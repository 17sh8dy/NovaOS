//! The user programs built into the kernel image (there is no file system
//! yet; stage 9 adds one). They come from `user/src/bin`, built by
//! tools/build_user.ps1 into `kernel/user_bin`.

macro_rules! program {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../user_bin/", $name, ".elf")) as &[u8])
    };
}

static PROGRAMS: &[(&str, &[u8])] = &[
    program!("hello"),
    program!("greet"),
    program!("counter"),
    program!("memtest"),
    program!("crash"),
    program!("evil"),
    program!("cat"),
    program!("ls"),
    program!("fsdemo"),
    program!("fsbig"),
];

/// The ELF image of the program called `name`.
pub fn find(name: &str) -> Option<&'static [u8]> {
    PROGRAMS.iter().find(|(n, _)| *n == name).map(|(_, image)| *image)
}

/// (name, size in bytes) of every built-in program.
pub fn list() -> impl Iterator<Item = (&'static str, usize)> {
    PROGRAMS.iter().map(|(n, image)| (*n, image.len()))
}
