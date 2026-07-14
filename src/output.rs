use std::{io, os::unix::ffi::OsStrExt};

use chrono::{DateTime, Local};
use nix::unistd::{Group, User};
use terminal_size::{Width, terminal_size};

use crate::{
    config::{Config, Format},
    list::EntryInfo,
};

pub trait OutputFormatter {
    fn write(&self, out: &mut dyn io::Write, entries: &[EntryInfo]) -> io::Result<()>;
}

pub struct LongFormatter {
    human_readable: bool,
}

impl LongFormatter {
    fn new(human_readable: bool) -> Self {
        Self { human_readable }
    }
}

impl OutputFormatter for LongFormatter {
    fn write(&self, out: &mut dyn io::Write, entries: &[EntryInfo]) -> io::Result<()> {
        let lines: Vec<_> = entries
            .iter()
            .map(|e| LongFormatLine::from_entry(e, self.human_readable))
            .collect();
        let col_widths = LongFormatColumnWidths::from_lines(&lines);

        writeln!(out, "total {}", total_kb_blocks(entries))?;
        for line in &lines {
            line.write(out, &col_widths)?;
        }

        Ok(())
    }
}

struct LongFormatLine {
    mode: String,
    nlink: String,
    user_name: String,
    group_name: String,
    size: String,
    modified: String,
    name: String,
}

impl LongFormatLine {
    fn write(
        &self,
        out: &mut dyn io::Write,
        col_widths: &LongFormatColumnWidths,
    ) -> io::Result<()> {
        let mode_width = col_widths.mode;
        let nlink_width = col_widths.nlink;
        let user_name_width = col_widths.user_name;
        let group_name_width = col_widths.group_name;
        let size_width = col_widths.size;
        let modified_width = col_widths.modified;

        writeln!(
            out,
            "{:>mode_width$} {:>nlink_width$} {:<user_name_width$} {:<group_name_width$} {:>size_width$} {:>modified_width$} {}",
            self.mode,
            self.nlink,
            self.user_name,
            self.group_name,
            self.size,
            self.modified,
            self.name,
        )
    }
}

impl LongFormatLine {
    fn from_entry(value: &EntryInfo, human_readable: bool) -> Self {
        let mut mode = String::new();
        if value.is_dir {
            mode.push('d');
        } else if value.is_symlink {
            mode.push('l');
        } else {
            mode.push('-');
        }

        let mut write_perm_flag = |c: char, mask: u32| {
            if value.permissions & mask > 0 {
                mode.push(c);
            } else {
                mode.push('-');
            }
        };
        write_perm_flag('r', 0o400);
        write_perm_flag('w', 0o200);
        write_perm_flag('x', 0o100);
        write_perm_flag('r', 0o40);
        write_perm_flag('w', 0o20);
        write_perm_flag('x', 0o10);
        write_perm_flag('r', 0o4);
        write_perm_flag('w', 0o2);
        write_perm_flag('x', 0o1);

        let uid = nix::unistd::Uid::from_raw(value.uid);
        let user_name = match User::from_uid(uid) {
            Ok(Some(user)) => user.name,
            _ => value.uid.to_string(),
        };
        let gid = nix::unistd::Gid::from_raw(value.gid);
        let group_name = match Group::from_gid(gid) {
            Ok(Some(group)) => group.name,
            _ => value.gid.to_string(),
        };

        let size = if human_readable {
            Self::format_size_for_human(value.size)
        } else {
            value.size.to_string()
        };

        let datetime: DateTime<Local> = value.modified_at.into();
        let now = Local::now();
        let is_old = now.signed_duration_since(datetime).num_days() > 180;

        let modified = if is_old {
            datetime.format("%b %e  %Y").to_string()
        } else {
            datetime.format("%b %e %H:%M").to_string()
        };

        let name = if let Some(symlink_target) = value.symlink_target.as_ref() {
            format!(
                "{} -> {}",
                value.name.to_string_lossy(),
                symlink_target.to_string_lossy()
            )
        } else {
            value.name.to_string_lossy().to_string()
        };

        Self {
            mode,
            nlink: value.nlink.to_string(),
            user_name,
            group_name,
            size,
            modified,
            name,
        }
    }

    fn format_size_for_human(size: u64) -> String {
        if size < 1024 {
            return size.to_string();
        }

        let mut size = size as f64;
        let units = ['K', 'M', 'G', 'T', 'P', 'E'];
        let mut units_iter = units.iter();
        let mut unit = units_iter.next();
        size /= 1024.0;

        while size > 1024.0 {
            size /= 1024.0;
            unit = units_iter.next();
        }

        if size < 10.0 {
            return format!("{:.1}{}", size, unit.expect("File too big"));
        }

        size = size.round();
        if size >= 1024.0 {
            size /= 1024.0;
            unit = units_iter.next();
            return format!("{:.1}{}", size, unit.expect("File too big"));
        }

        format!("{:.0}{}", size, unit.expect("File too big"))
    }
}

struct LongFormatColumnWidths {
    mode: usize,
    nlink: usize,
    user_name: usize,
    group_name: usize,
    size: usize,
    modified: usize,
}

impl LongFormatColumnWidths {
    fn from_lines(lines: &[LongFormatLine]) -> Self {
        let mode = lines.iter().map(|l| l.mode.len()).max().unwrap_or(0);
        let nlink = lines.iter().map(|l| l.nlink.len()).max().unwrap_or(0);
        let user_name = lines.iter().map(|l| l.user_name.len()).max().unwrap_or(0);
        let group_name = lines.iter().map(|l| l.group_name.len()).max().unwrap_or(0);
        let size = lines.iter().map(|l| l.size.len()).max().unwrap_or(0);
        let modified = lines.iter().map(|l| l.modified.len()).max().unwrap_or(0);

        Self {
            mode,
            nlink,
            user_name,
            group_name,
            size,
            modified,
        }
    }
}

fn total_kb_blocks(entries: &[EntryInfo]) -> u64 {
    entries.iter().map(|e| e.blocks_512).sum::<u64>() / 2
}

pub struct OneLineFormatter;

impl OutputFormatter for OneLineFormatter {
    fn write(&self, out: &mut dyn io::Write, entries: &[EntryInfo]) -> io::Result<()> {
        for entry in entries {
            out.write_all(entry.name.as_bytes())?;
            writeln!(out)?;
        }
        Ok(())
    }
}

pub struct ColumnsFormatter;

impl OutputFormatter for ColumnsFormatter {
    fn write(&self, out: &mut dyn io::Write, entries: &[EntryInfo]) -> io::Result<()> {
        let col_widths = calculate_columns_widths(entries).unwrap_or(vec![0]);
        write_columns(out, entries, &col_widths)
    }
}

fn write_columns(
    out: &mut dyn io::Write,
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
            out.write_all(entry.name.as_bytes())?;

            if col < cols_count - 1 {
                let padding = " ".repeat(col_width.saturating_sub(entry.name.len()) + 2);
                write!(out, "{}", padding)?;
            }
        }
        writeln!(out)?;
    }

    Ok(())
}

pub fn get_formatter(config: &Config) -> Box<dyn OutputFormatter> {
    match config.format {
        Format::OneLine => Box::new(OneLineFormatter),
        Format::Long => Box::new(LongFormatter::new(config.human_readable)),
        Format::Default => Box::new(ColumnsFormatter),
    }
}

fn calculate_columns_widths(entries: &[EntryInfo]) -> Option<Vec<usize>> {
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn one_line_formatter_writes_one_entry_per_line() {
        let entries: Vec<EntryInfo> = vec!["a", "b", "c"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        let mut out = Vec::new();
        OneLineFormatter.write(&mut out, &entries).unwrap();
        assert_eq!(out, b"a\nb\nc\n");
    }

    #[test]
    fn write_columns_pads_entries_to_column_width() {
        // 4 entries, 2 columns of widths [3, 5] → 2 rows
        // row 0: entries[0]="aa"  entries[2]="ccc"
        // row 1: entries[1]="b"   entries[3]="d"
        let entries: Vec<EntryInfo> = vec!["aa", "b", "ccc", "d"]
            .into_iter()
            .map(EntryInfo::from_name_only)
            .collect();
        let mut out = Vec::new();
        write_columns(&mut out, &entries, &[3, 5]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "aa   ccc\nb    d\n");
    }

    #[test]
    fn long_format_line_regular_file_permissions() {
        let entry = EntryInfo {
            permissions: 0o644,
            ..EntryInfo::from_name_only("file")
        };
        assert_eq!(LongFormatLine::from_entry(&entry, false).mode, "-rw-r--r--");
    }

    #[test]
    fn long_format_line_directory_type_char() {
        let entry = EntryInfo {
            is_dir: true,
            permissions: 0o755,
            ..EntryInfo::from_name_only("dir")
        };
        assert_eq!(LongFormatLine::from_entry(&entry, false).mode, "drwxr-xr-x");
    }

    #[test]
    fn long_format_line_symlink_shows_target() {
        let entry = EntryInfo {
            is_symlink: true,
            permissions: 0o777,
            symlink_target: Some(PathBuf::from("target")),
            ..EntryInfo::from_name_only("link")
        };
        let line = LongFormatLine::from_entry(&entry, false);
        assert_eq!(line.mode, "lrwxrwxrwx");
        assert_eq!(line.name, "link -> target");
    }

    #[test]
    fn long_format_line_columns_match_fields() {
        // uid/gid that (almost certainly) don't resolve to a real user/group,
        // so LongFormatLine falls back to their numeric representation.
        let entry = EntryInfo {
            nlink: 3,
            size: 1234,
            uid: u32::MAX,
            gid: u32::MAX,
            ..EntryInfo::from_name_only("file")
        };
        let line = LongFormatLine::from_entry(&entry, false);
        assert_eq!(line.nlink, "3");
        assert_eq!(line.size, "1234");
        assert_eq!(line.user_name, u32::MAX.to_string());
        assert_eq!(line.group_name, u32::MAX.to_string());
    }

    #[test]
    fn long_formatter_prints_total_in_1024_blocks() {
        let entries = vec![
            EntryInfo {
                blocks_512: 8,
                ..EntryInfo::from_name_only("a")
            },
            EntryInfo {
                blocks_512: 16,
                ..EntryInfo::from_name_only("b")
            },
        ];
        let mut out = Vec::new();
        LongFormatter::new(false).write(&mut out, &entries).unwrap();
        let out_str = String::from_utf8(out).unwrap();
        assert!(out_str.starts_with("total 12\n"));
    }

    #[test]
    fn format_size_for_human_below_1024_has_no_suffix() {
        assert_eq!(LongFormatLine::format_size_for_human(923), "923");
    }

    #[test]
    fn format_size_for_human_keeps_one_decimal_below_ten() {
        assert_eq!(LongFormatLine::format_size_for_human(4096), "4.0K");
    }

    #[test]
    fn format_size_for_human_drops_decimal_at_two_digits() {
        assert_eq!(LongFormatLine::format_size_for_human(10240), "10K");
    }

    #[test]
    fn format_size_for_human_advances_unit_at_exact_power_boundary() {
        assert_eq!(LongFormatLine::format_size_for_human(1_048_576), "1.0M");
        assert_eq!(LongFormatLine::format_size_for_human(1_073_741_824), "1.0G");
    }

    #[test]
    fn long_format_line_uses_human_readable_size_when_enabled() {
        let entry = EntryInfo {
            size: 1_048_576,
            ..EntryInfo::from_name_only("file")
        };
        assert_eq!(LongFormatLine::from_entry(&entry, true).size, "1.0M");
        assert_eq!(LongFormatLine::from_entry(&entry, false).size, "1048576");
    }
}
