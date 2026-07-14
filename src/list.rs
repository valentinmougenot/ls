use std::{
    cmp::Ordering,
    collections::VecDeque,
    ffi::OsString,
    fs::{DirEntry, Metadata},
    io::{self},
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    config::{Config, Format, HiddenMode, Sort},
    output::{OutputFormatter, get_formatter},
};

pub struct Lister<'a, W: io::Write> {
    config: &'a Config,
    out: W,
    formatter: Box<dyn OutputFormatter>,
}

impl<'a, W: io::Write> Lister<'a, W> {
    pub fn new(config: &'a Config, out: W) -> Self {
        let formatter = get_formatter(config);
        Self {
            config,
            out,
            formatter,
        }
    }

    pub fn list(&mut self) -> io::Result<()> {
        if self.config.list_dir {
            self.list_directory()?;
        } else {
            let mut queue: VecDeque<_> = self.config.paths.iter().map(PathBuf::from).collect();

            while let Some(path) = queue.pop_front() {
                if self.config.paths.len() > 1 || self.config.recursive {
                    self.out.write_all(path.as_os_str().as_bytes())?;
                    self.out.write_all(b":\n")?;
                }

                self.list_one(&path, &mut queue)?;

                if !queue.is_empty() {
                    self.out.write_all(b"\n")?;
                }
            }
        }

        Ok(())
    }

    fn list_one(&mut self, path: &Path, queue: &mut VecDeque<PathBuf>) -> io::Result<()> {
        let raw_entries: Vec<_> = match std::fs::read_dir(path) {
            Ok(entries) => entries
                .flatten()
                .filter(|e| {
                    if self.config.hidden_mode == HiddenMode::Default {
                        e.file_name().as_bytes().first() != Some(&b'.')
                    } else {
                        true
                    }
                })
                .collect(),
            Err(err) => {
                self.out.flush()?;
                eprintln!("{}", gnu_style_error(path, &err));
                return Err(err);
            }
        };

        if self.config.recursive {
            self.handle_recursion(path, &raw_entries, queue)?;
        }

        let mut entries: Vec<EntryInfo> = raw_entries
            .into_iter()
            .map(|e| EntryInfo::try_from_dir_entry(&e, self.config))
            .collect::<io::Result<Vec<EntryInfo>>>()?;

        if self.config.hidden_mode == HiddenMode::All {
            entries.push(EntryInfo::try_from_parent(
                path,
                OsString::from("."),
                self.config,
            )?);
            entries.push(EntryInfo::try_from_parent(
                path,
                OsString::from(".."),
                self.config,
            )?);
        }

        self.sort_entries(&mut entries);

        self.formatter.write(&mut self.out, &entries)
    }

    fn list_directory(&mut self) -> io::Result<()> {
        let mut entries: Vec<_> = self
            .config
            .paths
            .iter()
            .map(|e| EntryInfo::try_from_name(OsString::from(e), self.config))
            .collect::<io::Result<Vec<EntryInfo>>>()?;

        self.sort_entries(&mut entries);

        self.formatter.write(&mut self.out, &entries)
    }

    fn sort_entries(&self, entries: &mut [EntryInfo]) {
        entries.sort_by(|a, b| a.cmp_with_config(b, self.config));
    }

    fn handle_recursion(
        &self,
        path: &Path,
        raw_entries: &[DirEntry],
        queue: &mut VecDeque<PathBuf>,
    ) -> io::Result<()> {
        let mut sub_directories: Vec<_> = raw_entries
            .iter()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| EntryInfo::try_from_dir_entry(e, self.config))
            .collect::<io::Result<Vec<_>>>()?;

        sub_directories.sort_by(|a, b| a.cmp_with_config(b, self.config));
        for sub_dir in sub_directories.into_iter().rev() {
            queue.push_front(path.join(sub_dir.name));
        }

        Ok(())
    }
}

pub(crate) struct EntryInfo {
    pub name: OsString,
    pub modified_at: SystemTime,
    pub size: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<PathBuf>,
    pub permissions: u32,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub blocks_512: u64,
}

impl EntryInfo {
    fn try_from_dir_entry(value: &DirEntry, config: &Config) -> io::Result<Self> {
        Self::try_from_metadata(
            value.metadata()?,
            value.file_name().to_os_string(),
            value.path(),
            config,
        )
    }

    fn try_from_parent(parent: &Path, name: OsString, config: &Config) -> io::Result<Self> {
        let path = parent.join(&name);
        let metadata = path.symlink_metadata()?;
        Self::try_from_metadata(metadata, name, path, config)
    }

    fn try_from_name(name: OsString, config: &Config) -> io::Result<Self> {
        let path = PathBuf::from(&name);
        let metadata = path.symlink_metadata()?;
        Self::try_from_metadata(metadata, name, path, config)
    }

    fn try_from_metadata(
        metadata: Metadata,
        name: OsString,
        path: PathBuf,
        config: &Config,
    ) -> io::Result<Self> {
        let symlink_target = if metadata.is_symlink() && config.format == Format::Long {
            std::fs::read_link(&path).ok()
        } else {
            None
        };

        Ok(Self {
            name,
            modified_at: metadata.modified()?,
            size: metadata.len(),
            is_dir: metadata.is_dir(),
            is_symlink: metadata.is_symlink(),
            symlink_target,
            permissions: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            nlink: metadata.nlink(),
            blocks_512: metadata.blocks(),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_name_only(name: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            modified_at: SystemTime::UNIX_EPOCH,
            size: 0,
            is_dir: false,
            is_symlink: false,
            symlink_target: None,
            permissions: 0o644,
            uid: 0,
            gid: 0,
            nlink: 1,
            blocks_512: 0,
        }
    }

    #[cfg(test)]
    fn from_parts(name: impl Into<OsString>, modified_at: SystemTime) -> Self {
        Self {
            modified_at,
            ..Self::from_name_only(name)
        }
    }

    #[cfg(test)]
    fn from_size(name: impl Into<OsString>, size: u64) -> Self {
        Self {
            size,
            ..Self::from_name_only(name)
        }
    }

    fn cmp_with_config(&self, other: &EntryInfo, config: &Config) -> Ordering {
        let mut ordering = match config.sort {
            Sort::Time => other
                .modified_at
                .cmp(&self.modified_at)
                .then_with(|| self.name.as_bytes().cmp(other.name.as_bytes())),
            Sort::Size => other
                .size
                .cmp(&self.size)
                .then_with(|| self.name.as_bytes().cmp(other.name.as_bytes())),
            Sort::Default => self.name.as_bytes().cmp(other.name.as_bytes()),
        };

        if config.reverse {
            ordering = ordering.reverse();
        }

        ordering
    }
}

fn gnu_style_error(path: &Path, err: &io::Error) -> String {
    let reason = match err.kind() {
        io::ErrorKind::NotFound => "No such file or directory",
        io::ErrorKind::PermissionDenied => "Permission denied",
        io::ErrorKind::NotADirectory => "Not a directory",
        _ => "Unknown error",
    };
    format!("ls: cannot access '{}': {}", path.display(), reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    use crate::config::{Config, Format, HiddenMode, Sort};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("ls_tests_{}", name));
            fs::remove_dir_all(&path).ok();
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str) -> &Self {
            fs::write(self.0.join(name), "").unwrap();
            self
        }

        fn dir(&self, name: &str) -> &Self {
            fs::create_dir_all(self.0.join(name)).unwrap();
            self
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn make_config(
        format: Format,
        hidden_mode: HiddenMode,
        reverse: bool,
        recursive: bool,
        paths: Vec<PathBuf>,
    ) -> Config {
        Config {
            format,
            hidden_mode,
            reverse,
            list_dir: false,
            recursive,
            sort: Sort::Default,
            paths,
        }
    }

    fn run(config: &Config) -> String {
        let mut out = Vec::new();
        Lister::new(config, &mut out).list().unwrap();
        String::from_utf8(out).unwrap()
    }

    fn time_config(reverse: bool, paths: Vec<PathBuf>) -> Config {
        Config {
            format: Format::OneLine,
            hidden_mode: HiddenMode::Default,
            reverse,
            list_dir: false,
            recursive: false,
            sort: Sort::Time,
            paths,
        }
    }

    #[test]
    fn sort_entries_alphabetical() {
        let c = make_config(Format::Default, HiddenMode::Default, false, false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let mut entries: Vec<EntryInfo> = vec!["c", "a", "b"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn sort_entries_reverse() {
        let c = make_config(Format::Default, HiddenMode::Default, true, false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let mut entries: Vec<EntryInfo> = vec!["c", "a", "b"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn list_alphabetical_order() {
        let dir = TempDir::new("alphabetical");
        dir.file("c").file("a").file("b");
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            false,
            false,
            vec![dir.path()],
        );
        assert_eq!(run(&c), "a\nb\nc\n");
    }

    #[test]
    fn list_reverse_order() {
        let dir = TempDir::new("reverse");
        dir.file("a").file("b").file("c");
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            true,
            false,
            vec![dir.path()],
        );
        assert_eq!(run(&c), "c\nb\na\n");
    }

    #[test]
    fn list_excludes_hidden_by_default() {
        let dir = TempDir::new("hidden_default");
        dir.file("visible").file(".hidden");
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            false,
            false,
            vec![dir.path()],
        );
        assert_eq!(run(&c), "visible\n");
    }

    #[test]
    fn list_all_includes_hidden_and_dots() {
        let dir = TempDir::new("hidden_all");
        dir.file("visible").file(".hidden");
        let c = make_config(
            Format::OneLine,
            HiddenMode::All,
            false,
            false,
            vec![dir.path()],
        );
        assert_eq!(run(&c), ".\n..\n.hidden\nvisible\n");
    }

    #[test]
    fn list_almost_all_includes_hidden_not_dots() {
        let dir = TempDir::new("hidden_almost_all");
        dir.file("visible").file(".hidden");
        let c = make_config(
            Format::OneLine,
            HiddenMode::AlmostAll,
            false,
            false,
            vec![dir.path()],
        );
        assert_eq!(run(&c), ".hidden\nvisible\n");
    }

    #[test]
    fn list_multiple_paths_shows_headers() {
        let dir1 = TempDir::new("multi1");
        let dir2 = TempDir::new("multi2");
        dir1.file("x");
        dir2.file("y");
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            false,
            false,
            vec![dir1.path(), dir2.path()],
        );
        let out = run(&c);
        let sections: Vec<&str> = out.split("\n\n").collect();
        assert_eq!(sections.len(), 2);
        assert!(sections[0].contains(":\n") && sections[0].contains("x"));
        assert!(sections[1].contains(":\n") && sections[1].contains("y"));
    }

    #[test]
    fn list_recursive_visits_subdirectories() {
        let dir = TempDir::new("recursive");
        dir.file("root_file").dir("subdir");
        fs::write(dir.path().join("subdir/sub_file"), "").unwrap();
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            false,
            true,
            vec![dir.path()],
        );
        let out = run(&c);
        assert!(out.contains("root_file\n"));
        assert!(out.contains("subdir\n"));
        assert!(out.contains("sub_file\n"));
    }

    #[test]
    fn sort_by_time_newest_first() {
        let c = time_config(false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + std::time::Duration::from_secs(1);
        let t2 = t0 + std::time::Duration::from_secs(2);
        let mut entries = vec![
            EntryInfo::from_parts("a", t0),
            EntryInfo::from_parts("b", t2),
            EntryInfo::from_parts("c", t1),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn sort_by_time_tiebreaker_alphabetical() {
        let c = time_config(false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let t = SystemTime::UNIX_EPOCH;
        let mut entries = vec![
            EntryInfo::from_parts("c", t),
            EntryInfo::from_parts("a", t),
            EntryInfo::from_parts("b", t),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn sort_by_time_reverse() {
        let c = time_config(true, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + std::time::Duration::from_secs(1);
        let t2 = t0 + std::time::Duration::from_secs(2);
        let mut entries = vec![
            EntryInfo::from_parts("a", t0),
            EntryInfo::from_parts("b", t2),
            EntryInfo::from_parts("c", t1),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a", "c", "b"]);
    }

    #[test]
    fn list_sort_by_time_newest_first() {
        let dir = TempDir::new("sort_time");
        dir.file("a");
        std::thread::sleep(std::time::Duration::from_millis(10));
        dir.file("b");
        assert_eq!(run(&time_config(false, vec![dir.path()])), "b\na\n");
    }

    fn size_config(reverse: bool, paths: Vec<PathBuf>) -> Config {
        Config {
            format: Format::OneLine,
            hidden_mode: HiddenMode::Default,
            reverse,
            list_dir: false,
            recursive: false,
            sort: Sort::Size,
            paths,
        }
    }

    #[test]
    fn sort_by_size_largest_first() {
        let c = size_config(false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let mut entries = vec![
            EntryInfo::from_size("small", 1),
            EntryInfo::from_size("large", 100),
            EntryInfo::from_size("medium", 50),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["large", "medium", "small"]);
    }

    #[test]
    fn sort_by_size_tiebreaker_alphabetical() {
        let c = size_config(false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let mut entries = vec![
            EntryInfo::from_size("c", 10),
            EntryInfo::from_size("a", 10),
            EntryInfo::from_size("b", 10),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn sort_by_size_reverse() {
        let c = size_config(true, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let mut entries = vec![
            EntryInfo::from_size("small", 1),
            EntryInfo::from_size("large", 100),
            EntryInfo::from_size("medium", 50),
        ];
        lister.sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
        assert_eq!(names, vec!["small", "medium", "large"]);
    }

    #[test]
    fn list_sort_by_size_largest_first() {
        let dir = TempDir::new("sort_size");
        fs::write(dir.path().join("small"), "x").unwrap();
        fs::write(dir.path().join("large"), "x".repeat(100).as_str()).unwrap();
        assert_eq!(run(&size_config(false, vec![dir.path()])), "large\nsmall\n");
    }

    fn long_config(paths: Vec<PathBuf>) -> Config {
        Config {
            format: Format::Long,
            hidden_mode: HiddenMode::Default,
            reverse: false,
            list_dir: false,
            recursive: false,
            sort: Sort::Default,
            paths,
        }
    }

    #[test]
    fn list_long_format_shows_permissions_and_total() {
        let dir = TempDir::new("long_format");
        dir.file("a");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.path().join("a"), fs::Permissions::from_mode(0o644)).unwrap();

        let out = run(&long_config(vec![dir.path()]));
        let lines: Vec<&str> = out.lines().collect();

        assert!(lines[0].starts_with("total "));
        assert!(
            lines[0]
                .strip_prefix("total ")
                .unwrap()
                .parse::<u64>()
                .is_ok()
        );
        assert!(lines[1].contains("-rw-r--r--"));
        assert!(lines[1].trim_end().ends_with('a'));
    }

    #[test]
    fn list_long_format_shows_symlink_target() {
        let dir = TempDir::new("symlink");
        dir.file("target");
        std::os::unix::fs::symlink(dir.path().join("target"), dir.path().join("link")).unwrap();

        let out = run(&long_config(vec![dir.path()]));
        assert!(out.contains("link -> "));
        assert!(out.lines().any(|l| l.starts_with('l')));
    }

    #[test]
    fn list_nonexistent_path_returns_error() {
        let c = make_config(
            Format::OneLine,
            HiddenMode::Default,
            false,
            false,
            vec![PathBuf::from("/nonexistent_ls_test_path")],
        );
        let mut out = Vec::new();
        assert!(Lister::new(&c, &mut out).list().is_err());
    }
}
