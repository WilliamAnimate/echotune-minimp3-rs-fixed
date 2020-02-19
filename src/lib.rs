extern crate minimp3_sys as ffi;
extern crate slice_deque;

use slice_deque::SliceDeque;
use std::io::{self, Read, Seek};
use std::mem;
use std::os::raw::{c_int, c_void};

mod error;
use error::from_mini_error;
pub use error::Error;

/// Maximum number of samples present in a MP3 frame.
pub const MAX_SAMPLES_PER_FRAME: usize = ffi::MINIMP3_MAX_SAMPLES_PER_FRAME as usize;

const BUFFER_SIZE: usize = MAX_SAMPLES_PER_FRAME * 15;
const REFILL_TRIGGER: usize = MAX_SAMPLES_PER_FRAME * 8;

/// A MP3 decoder which consumes a reader and produces [`Frame`]s.
///
/// [`Frame`]: ./struct.Frame.html
pub struct Decoder<R> {
    reader: R,
    buffer: SliceDeque<u8>,
    decoder: Box<ffi::mp3dec_t>,
}

/// A MP3 frame, owning the decoded audio of that frame.
pub struct Frame {
    /// The decoded audio held by this frame. Channels are interleaved.
    pub data: Vec<i16>,
    /// This frame's sample rate in hertz.
    pub sample_rate: i32,
    /// The number of channels in this frame.
    pub channels: usize,
    /// MPEG layer used by this file.
    pub layer: usize,
    /// Current bitrate as of this frame, in kb/s.
    pub bitrate: i32,
}

impl<R> Decoder<R>
where
    R: Read,
{
    /// Creates a new decoder, consuming the `reader`.
    pub fn new(reader: R) -> Decoder<R> {
        let mut minidec = unsafe { Box::new(mem::zeroed()) };
        unsafe { ffi::mp3dec_init(&mut *minidec) }

        Decoder {
            reader,
            buffer: SliceDeque::with_capacity(BUFFER_SIZE),
            decoder: minidec,
        }
    }

    /// Reads a new frame from the internal reader. Returns a [`Frame`] if one was found,
    /// or, otherwise, an `Err` explaining why not.
    ///
    /// [`Frame`]: ./struct.Frame.html
    pub fn next_frame(&mut self) -> Result<Frame, Error> {
        loop {
            // Keep our buffers full
            let bytes_read = if self.buffer.len() < REFILL_TRIGGER {
                Some(self.refill()?)
            } else {
                None
            };

            match self.decode_frame() {
                Ok(frame) => return Ok(frame),
                // Don't do anything if we didn't have enough data or we skipped data,
                // just let the loop spin around another time.
                Err(Error::InsufficientData) | Err(Error::SkippedData) => {
                    // If there are no more bytes to be read from the file, return EOF
                    if let Some(0) = bytes_read {
                        return Err(Error::Eof);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn reader(&self) -> &R {
        &self.reader
    }

    pub fn reader_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    fn decode_frame(&mut self) -> Result<Frame, Error> {
        let mut frame_info = unsafe { mem::zeroed() };
        let mut pcm = Vec::with_capacity(MAX_SAMPLES_PER_FRAME);
        let samples: usize = unsafe {
            ffi::mp3dec_decode_frame(
                &mut *self.decoder,
                self.buffer.as_ptr(),
                self.buffer.len() as _,
                pcm.as_mut_ptr(),
                &mut frame_info,
            ) as _
        };

        if samples > 0 {
            unsafe {
                pcm.set_len(samples * frame_info.channels as usize);
            }
        }

        let frame = Frame {
            data: pcm,
            sample_rate: frame_info.hz,
            channels: frame_info.channels as usize,
            layer: frame_info.layer as usize,
            bitrate: frame_info.bitrate_kbps,
        };

        let current_len = self.buffer.len();
        self.buffer
            .truncate_front(current_len - frame_info.frame_bytes as usize);

        if samples == 0 {
            if frame_info.frame_bytes > 0 {
                Err(Error::SkippedData)
            } else {
                Err(Error::InsufficientData)
            }
        } else {
            Ok(frame)
        }
    }

    fn refill(&mut self) -> Result<usize, io::Error> {
        let mut dat: [u8; MAX_SAMPLES_PER_FRAME * 5] = [0; MAX_SAMPLES_PER_FRAME * 5];
        let read_bytes = self.reader.read(&mut dat)?;
        self.buffer.extend(dat[..read_bytes].iter());

        Ok(read_bytes)
    }
}

// Need to box this to avoid pointers being invalidated due to movement
struct SeekDecoderInner<R> {
    reader: R,
    mini_ex_io: ffi::mp3dec_io_t,
    mini_ex_dec: ffi::mp3dec_ex_t,
}

/// A sample level Seekable MP3 decoder which consumes a reader and produces samples.
///
/// Unlike `Decoder` this requires `Seek` + `Read`. Also when possible, depending on the mp3 encoder this will trim of samples that were added as part of the encoding process.
pub struct SeekDecoder<R>(Box<SeekDecoderInner<R>>);

unsafe extern "C" fn read_cb<R>(buf: *mut c_void, size: usize, user_data: *mut c_void) -> usize
where
    R: Read,
{
    // Not sure how to safely panic from within callback
    let reader = &mut *(user_data as *mut R);
    let buf = std::slice::from_raw_parts_mut(buf as *mut u8, size as usize);
    let mut position = 0;
    // Mimic fread call where we return
    // -1 for error
    // size for not end of stream/file
    // 0 or less than size for end of stream/file
    while position < size as usize {
        match reader.read(&mut buf[position..]) {
            Ok(n) if n == 0 => return position,
            Ok(n) => position += n,
            // -1
            Err(_) => return std::usize::MAX,
        }
    }
    position
}

unsafe extern "C" fn seek_cb<S>(position: u64, user_data: *mut c_void) -> c_int
where
    S: Seek,
{
    use std::io::SeekFrom;
    let seeker = &mut *(user_data as *mut S);
    match seeker.seek(SeekFrom::Start(position)) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

impl<R> SeekDecoder<R>
where
    R: Read + Seek,
{
    /// Creates a new `SeekDecoder`, consuming the `reader`.
    pub fn new(reader: R) -> Result<SeekDecoder<R>, Error> {
        // Need to lock reader's memory location before setting callbacks
        // Need to lock io's memory location before opening decoder
        let mut inner = Box::new(SeekDecoderInner {
            reader,
            mini_ex_io: unsafe { mem::zeroed() },
            mini_ex_dec: unsafe { mem::zeroed() },
        });

        inner.mini_ex_io.read = Some(read_cb::<R>);
        inner.mini_ex_io.read_data = &mut inner.reader as *mut _ as *mut c_void;
        inner.mini_ex_io.seek = Some(seek_cb::<R>);
        inner.mini_ex_io.seek_data = &mut inner.reader as *mut _ as *mut c_void;

        let res = unsafe {
            ffi::mp3dec_ex_open_cb(
                &mut inner.mini_ex_dec,
                &mut inner.mini_ex_io,
                ffi::MP3D_SEEK_TO_SAMPLE as i32,
            )
        };
        from_mini_error(res)?;
        Ok(SeekDecoder(inner))
    }

    /// This mp3s sample rate in hertz.
    pub fn sample_rate(&self) -> i32 {
        self.0.mini_ex_dec.info.hz
    }
    /// The number of channels in this mp3.
    pub fn channels(&self) -> usize {
        self.0.mini_ex_dec.info.channels as usize
    }

    /// Returns the number of samples that were set
    /// Will be zero at end of stream
    pub fn read_samples(&mut self, buf: &mut [i16]) -> Result<usize, Error> {
        let len = unsafe {
            ffi::mp3dec_ex_read(&mut self.0.mini_ex_dec, buf.as_mut_ptr(), buf.len()) as usize
        };

        if len == buf.len() {
            Ok(len)
        } else if len < buf.len() {
            // Check for error occurred
            from_mini_error(self.0.mini_ex_dec.last_error)?;
            // Must be end of stream
            Ok(len)
        } else {
            panic!("Minimp3 returned invalid read result. Likely corrupt memory")
        }
    }

    /// Convenience wrapper around `read_samples` to use with a while let loop
    /// Returns None when out of samples
    /// Returns the slice of newly assigned samples otherwise
    pub fn read_sample_slice<'a>(
        &mut self,
        buf: &'a mut [i16],
    ) -> Result<Option<&'a mut [i16]>, Error> {
        let len = self.read_samples(buf)?;
        Ok(if len == 0 {
            None
        } else {
            Some(&mut buf[..len])
        })
    }

    /// Seek to the given sample index
    pub fn seek_samples(&mut self, sample: u64) -> Result<(), Error> {
        let res = unsafe { ffi::mp3dec_ex_seek(&mut self.0.mini_ex_dec, sample) };
        from_mini_error(res)
    }
}
