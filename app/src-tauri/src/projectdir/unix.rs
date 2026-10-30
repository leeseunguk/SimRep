//! Directory-relative mutation: a replaced pathname cannot redirect an open directory handle.
use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

#[cfg(test)]
#[path = "security_tests.rs"]
mod security_tests;

fn name(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
}

pub(super) struct Directory(File);

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Self> {
        let mut dir = Self(File::open("/")?);
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(part) => {
                    let part = name(part)?;
                    if create {
                        // mkdirat never follows the final component; openat below verifies it.
                        let result =
                            unsafe { libc::mkdirat(dir.0.as_raw_fd(), part.as_ptr(), 0o700) };
                        if result < 0
                            && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists
                        {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    dir = dir.child(&part)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "non-absolute directory path",
                    ));
                }
            }
        }
        Ok(dir)
    }

    fn child(&self, part: &CString) -> io::Result<Self> {
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }

    pub(super) fn verify(&self, path: &Path) -> io::Result<()> {
        let current = Self::open(path, false)?;
        let a = self.0.metadata()?;
        let b = current.0.metadata()?;
        if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
            return Err(io::Error::other(
                "project directory changed during operation",
            ));
        }
        Ok(())
    }

    pub(super) fn create_temp(&self, file: &OsStr) -> io::Result<File> {
        let file = name(file)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                file.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub(super) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
        let from = name(from)?;
        let to = name(to)?;
        let result = unsafe {
            libc::renameat(
                self.0.as_raw_fd(),
                from.as_ptr(),
                self.0.as_raw_fd(),
                to.as_ptr(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn remove(&self, file: &OsStr) -> io::Result<()> {
        self.remove_child(&name(file)?)
    }

    fn remove_child(&self, file: &CString) -> io::Result<()> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                file.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        let is_dir = stat.st_mode & libc::S_IFMT == libc::S_IFDIR;
        if is_dir {
            let child = self.child(file)?;
            for entry in child.names()? {
                child.remove_child(&name(&entry)?)?;
            }
        }
        let flags = if is_dir { libc::AT_REMOVEDIR } else { 0 };
        if unsafe { libc::unlinkat(self.0.as_raw_fd(), file.as_ptr(), flags) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn names(&self) -> io::Result<Vec<OsString>> {
        let fd = unsafe { libc::fcntl(self.0.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            return Err(io::Error::last_os_error());
        }
        let mut names = Vec::new();
        loop {
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                break;
            }
            let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes != b"." && bytes != b".." {
                names.push(OsString::from_vec(bytes.to_vec()));
            }
        }
        unsafe { libc::closedir(stream) };
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn replacement_directory_cannot_redirect_mutations() {
        let root = std::env::temp_dir().join(format!("simrep-dir-handle-{}", std::process::id()));
        std::fs::create_dir_all(root.join("inside")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        let inside = root.join("inside");
        let outside = root.join("outside");
        std::fs::write(inside.join("delete-me"), b"inside").unwrap();
        std::fs::write(outside.join("delete-me"), b"KEEP").unwrap();
        let dir = Directory::open(&inside, false).unwrap();
        std::fs::rename(&inside, root.join("moved")).unwrap();
        std::os::unix::fs::symlink(&outside, &inside).unwrap();
        assert!(dir.verify(&inside).is_err());
        let mut file = dir.create_temp(OsStr::new("new.tmp")).unwrap();
        file.write_all(b"SAFE").unwrap();
        dir.rename(OsStr::new("new.tmp"), OsStr::new("new.json"))
            .unwrap();
        dir.remove(OsStr::new("delete-me")).unwrap();
        assert!(!outside.join("new.json").exists());
        assert_eq!(std::fs::read(outside.join("delete-me")).unwrap(), b"KEEP");
        assert_eq!(std::fs::read(root.join("moved/new.json")).unwrap(), b"SAFE");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusive_temporary_creation_refuses_a_preexisting_link() {
        let root = std::env::temp_dir().join(format!("simrep-exclusive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sentinel"), b"KEEP").unwrap();
        std::os::unix::fs::symlink(root.join("sentinel"), root.join("new.tmp")).unwrap();
        let dir = Directory::open(&root, false).unwrap();
        assert!(dir.create_temp(OsStr::new("new.tmp")).is_err());
        assert_eq!(std::fs::read(root.join("sentinel")).unwrap(), b"KEEP");
        std::fs::remove_dir_all(root).unwrap();
    }
}
