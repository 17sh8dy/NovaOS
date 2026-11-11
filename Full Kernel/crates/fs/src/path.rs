//! Path handling: absolute paths only, `.` and `..` resolved lexically.

use crate::{FsError, Result};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Longest single name NovaFS can store.
pub const MAX_NAME: usize = 58;

/// Splits an absolute path into components, resolving `.` and `..` (`..` at
/// the root stays at the root). `"/"` gives no components.
pub fn components(path: &str) -> Result<Vec<String>> {
    if !path.starts_with('/') {
        return Err(FsError::InvalidArgument);
    }
    let mut out: Vec<String> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            name => {
                check_name(name)?;
                out.push(name.to_string());
            }
        }
    }
    Ok(out)
}

/// A name usable in a directory: 1..=58 bytes, no `/`, no NUL, not `.` or `..`.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(FsError::InvalidName);
    }
    if name.len() > MAX_NAME {
        return Err(FsError::NameTooLong);
    }
    Ok(())
}

/// The canonical text of a component list (`/a/b`, or `/` when empty).
pub fn join(components: &[String]) -> String {
    if components.is_empty() {
        return String::from("/");
    }
    let mut s = String::new();
    for c in components {
        s.push('/');
        s.push_str(c);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comps(p: &str) -> Vec<String> {
        components(p).unwrap()
    }

    #[test]
    fn splits_and_normalises() {
        assert!(comps("/").is_empty());
        assert_eq!(comps("/a/b"), ["a", "b"]);
        assert_eq!(comps("//a///b/"), ["a", "b"]);
        assert_eq!(comps("/a/./b/../c"), ["a", "c"]);
        assert_eq!(comps("/../.."), Vec::<String>::new(), ".. stops at the root");
    }

    #[test]
    fn rejects_relative_paths_and_bad_names() {
        assert_eq!(components("a/b"), Err(FsError::InvalidArgument));
        assert_eq!(components(""), Err(FsError::InvalidArgument));
        assert_eq!(components("/a/\0"), Err(FsError::InvalidName));
        let long = "x".repeat(MAX_NAME + 1);
        assert_eq!(components(&alloc::format!("/{long}")), Err(FsError::NameTooLong));
        assert!(components(&alloc::format!("/{}", "x".repeat(MAX_NAME))).is_ok());
    }

    #[test]
    fn join_is_the_inverse() {
        assert_eq!(join(&[]), "/");
        assert_eq!(join(&comps("/a/b/../c")), "/a/c");
    }

    #[test]
    fn name_checks() {
        assert!(check_name("hello.txt").is_ok());
        assert_eq!(check_name(""), Err(FsError::InvalidName));
        assert_eq!(check_name(".."), Err(FsError::InvalidName));
        assert_eq!(check_name("a/b"), Err(FsError::InvalidName));
    }
}
