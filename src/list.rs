use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::DirEntry,
    io::{self},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use terminal_size::{Width, terminal_size};

use crate::config::{Config, Format, HiddenMode};

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
            self.handle_recursion(path, &raw_entries, queue);
        }

        let mut entries: Vec<_> = raw_entries.into_iter().map(|e| e.file_name()).collect();

        if self.config.hidden_mode == HiddenMode::All {
            entries.push(OsString::from("."));
            entries.push(OsString::from(".."));
        }

        self.sort_entries(&mut entries);

        self.write_output(&entries)
    }

    fn list_directory(&mut self) -> io::Result<()> {
        let mut entries: Vec<_> = self.config.paths.iter().map(OsString::from).collect();
        self.sort_entries(&mut entries);
        self.write_output(&entries)
    }

    fn sort_entries(&self, entries: &mut [OsString]) {
        if self.config.reverse {
            entries.sort_by(|a, b| b.as_bytes().cmp(a.as_bytes()));
        } else {
            entries.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        }
    }

    fn calculate_columns_widths(&self, entries: &[OsString]) -> Option<Vec<usize>> {
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
                    col_widths[col] = col_widths[col].max(entry.len());
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

    fn write_output(&mut self, entries: &[OsString]) -> io::Result<()> {
        match self.calculate_columns_widths(entries) {
            Some(values) => self.write_multi_columns_output(entries, &values),
            None => self.write_one_column_output(entries),
        }
    }

    fn write_one_column_output(&mut self, entries: &[OsString]) -> io::Result<()> {
        for entry in entries {
            self.out.write_all(entry.as_bytes())?;
            self.out.write_all(b"\n")?;
        }

        Ok(())
    }

    fn write_multi_columns_output(
        &mut self,
        entries: &[OsString],
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
                self.out.write_all(entry.as_bytes())?;

                if col < cols_count - 1 {
                    let padding = " ".repeat(col_width.saturating_sub(entry.len()) + 2);
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
    ) {
        let mut sub_directories: Vec<_> = raw_entries
            .iter()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| path.to_owned().join(e.file_name()))
            .collect();

        sub_directories.sort();
        if self.config.reverse {
            for sub_dir in sub_directories {
                queue.push_front(sub_dir);
            }
        } else {
            for sub_dir in sub_directories.into_iter().rev() {
                queue.push_front(sub_dir);
            }
        }
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
