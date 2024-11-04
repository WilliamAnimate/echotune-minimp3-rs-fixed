fn main() {
    println!("cargo:rustc-link-lib=minimp3");
} // This is needed for AUR, cargo will go wild if this isn't here.
