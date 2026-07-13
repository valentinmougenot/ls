use std::{
    cmp::Ordering,
    collections::VecDeque,
    ffi::OsString,
    fs::{DirEntry, Metadata},
    io::{self},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use terminal_size::{Width, terminal_size};

use crate::config::{Config, Format, HiddenMode, Sort};

pub struct Lister<'a, W: io::Write> {
    config: &'a Config,
    out: W,
}

impl<'a, W: io::Write> Lister<'a, W> {
    pub fn new(config: &'a Config, out: W) -> Self {
        Self { config, out }
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
                eprintln!("{}", gnu_style_error(path, &err));
                return Err(err);
            }
        };

        if self.config.recursive {
            self.handle_recursion(path, &raw_entries, queue)?;
        }

        let mut entries: Vec<EntryInfo> = raw_entries
            .into_iter()
            .map(EntryInfo::try_from)
            .collect::<io::Result<Vec<EntryInfo>>>()?;

        if self.config.hidden_mode == HiddenMode::All {
            entries.push(EntryInfo::try_from_parent(path, OsString::from("."))?);
            entries.push(EntryInfo::try_from_parent(path, OsString::from(".."))?);
        }

        self.sort_entries(&mut entries);

        self.write_output(&entries)
    }

    fn list_directory(&mut self) -> io::Result<()> {
        let mut entries: Vec<_> = self
            .config
            .paths
            .iter()
            .map(|e| EntryInfo::try_from_name(OsString::from(e)))
            .collect::<io::Result<Vec<EntryInfo>>>()?;

        self.sort_entries(&mut entries);

        self.write_output(&entries)
    }

    fn sort_entries(&self, entries: &mut [EntryInfo]) {
        entries.sort_by(|a, b| a.cmp_with_config(b, self.config));
    }

    fn calculate_columns_widths(&self, entries: &[EntryInfo]) -> Option<Vec<usize>> {
        if self.config.format == Format::OneLine {
            return None;
        }

        let mut col_count = entries.len();
        let mut fits = false;

        if let Some((Width(term_width), _)) = terminal_size() {
            while !fits && col_count > 1 {
                let mut col_widths = vec![0; col_count];
                let rows_count = entries.len().div_ceil(col_count);

                for (i, entry) in entries.iter().enumerate() {
                    let col = i / rows_count;
                    col_widths[col] = col_widths[col].max(entry.name.len());
                }

                let total_width = col_widths.iter().copied().sum::<usize>() + 2 * (col_count - 1);
                fits = total_width <= term_width as usize;

                if !fits {
                    col_count -= 1;
                } else {
                    col_widths.retain(|&c| c != 0);
                    return Some(col_widths);
                }
            }
        }

        None
    }

    fn write_output(&mut self, entries: &[EntryInfo]) -> io::Result<()> {
        match self.calculate_columns_widths(entries) {
            Some(values) => self.write_multi_columns_output(entries, &values),
            None => self.write_one_column_output(entries),
        }
    }

    fn write_one_column_output(&mut self, entries: &[EntryInfo]) -> io::Result<()> {
        for entry in entries {
            entry.write(&mut self.out)?;
            self.out.write_all(b"\n")?;
        }

        Ok(())
    }

    fn write_multi_columns_output(
        &mut self,
        entries: &[EntryInfo],
        col_widths: &[usize],
    ) -> io::Result<()> {
        let cols_count = col_widths.len();
        let rows_count = entries.len().div_ceil(cols_count);

        for row in 0..rows_count {
            for (col, col_width) in col_widths.iter().enumerate() {
                let idx = col * rows_count + row;

                if idx >= entries.len() {
                    break;
                }

                let entry = &entries[idx];
                entry.write(&mut self.out)?;

                if col < cols_count - 1 {
                    let padding = " ".repeat(col_width.saturating_sub(entry.name.len()) + 2);
                    self.out.write_all(padding.as_bytes())?;
                }
            }
            self.out.write_all(b"\n")?;
        }

        Ok(())
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
            .map(EntryInfo::try_from)
            .collect::<io::Result<Vec<_>>>()?;

        sub_directories.sort_by(|a, b| a.cmp_with_config(b, self.config));
        for sub_dir in sub_directories.into_iter().rev() {
            queue.push_front(path.join(sub_dir.name));
        }

        Ok(())
    }
}

struct EntryInfo {
    name: OsString,
    modified_at: SystemTime,
}

impl TryFrom<DirEntry> for EntryInfo {
    type Error = io::Error;

    fn try_from(value: DirEntry) -> Result<Self, Self::Error> {
        Self::try_from_metadata(value.metadata()?, value.file_name().to_os_string())
    }
}

impl TryFrom<&DirEntry> for EntryInfo {
    type Error = io::Error;

    fn try_from(value: &DirEntry) -> Result<Self, Self::Error> {
        Self::try_from_metadata(value.metadata()?, value.file_name().to_os_string())
    }
}

impl EntryInfo {
    fn try_from_parent(parent: &Path, name: OsString) -> io::Result<Self> {
        let metadata = parent.join(&name).metadata()?;
        Self::try_from_metadata(metadata, name)
    }

    fn try_from_name(name: OsString) -> io::Result<Self> {
        let metadata = PathBuf::from(&name).metadata()?;
        Self::try_from_metadata(metadata, name)
    }

    fn try_from_metadata(metadata: Metadata, name: OsString) -> io::Result<Self> {
        Ok(Self {
            name,
            modified_at: metadata.modified()?,
        })
    }

    fn write<W: io::Write>(&self, out: &mut W) -> io::Result<()> {
        out.write_all(self.name.as_bytes())
    }

    #[cfg(test)]
    fn from_name_only(name: impl Into<OsString>) -> Self {
        Self {
            name: name.into(),
            modified_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[cfg(test)]
    fn from_parts(name: impl Into<OsString>, modified_at: SystemTime) -> Self {
        Self { name: name.into(), modified_at }
    }

    fn cmp_with_config(&self, other: &EntryInfo, config: &Config) -> Ordering {
        let mut ordering = match config.sort {
            Sort::Time => other
                .modified_at
                .cmp(&self.modified_at)
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
    fn one_column_output() {
        let c = make_config(Format::OneLine, HiddenMode::Default, false, false, vec![]);
        let mut out = Vec::new();
        let mut lister = Lister::new(&c, &mut out);
        let entries: Vec<EntryInfo> = vec!["a", "b", "c"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        lister.write_one_column_output(&entries).unwrap();
        assert_eq!(out, b"a\nb\nc\n");
    }

    #[test]
    fn multi_column_output() {
        // 4 entries, 2 columns of widths [3, 5] → 2 rows
        // row 0: entries[0]="aa"  entries[2]="ccc"
        // row 1: entries[1]="b"   entries[3]="d"
        let c = make_config(Format::Default, HiddenMode::Default, false, false, vec![]);
        let mut out = Vec::new();
        let mut lister = Lister::new(&c, &mut out);
        let entries: Vec<EntryInfo> = vec!["aa", "b", "ccc", "d"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        lister
            .write_multi_columns_output(&entries, &[3, 5])
            .unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "aa   ccc\nb    d\n");
    }

    #[test]
    fn columns_widths_returns_none_for_one_line_format() {
        let c = make_config(Format::OneLine, HiddenMode::Default, false, false, vec![]);
        let lister = Lister::new(&c, Vec::<u8>::new());
        let entries: Vec<EntryInfo> = vec!["a", "b"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        assert_eq!(lister.calculate_columns_widths(&entries), None);
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
