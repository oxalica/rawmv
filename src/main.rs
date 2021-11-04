// SPDX-License-Identifier: GPL-3.0-only
use std::convert::TryInto;
use std::ffi::CString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::{fs, process};
use structopt::clap::{Error as ClapError, ErrorKind as ClapErrorKind};
use structopt::StructOpt;

#[derive(Debug)]
struct App {
    force: bool,
    no_clobber: bool,
    interactive: bool,
    verbose: bool,
    operations: Vec<(PathBuf, PathBuf)>,
}

/// mv(1) but without cp(1) fallback. Simple wrapper of rename(2)/renameat2(2).
#[derive(StructOpt, Debug)]
#[structopt(
    usage = "\
    rawmv [OPTION]... [-T] <SOURCE> <DEST>
    rawmv [OPTION]... <SOURCE>... <DIRECTORY>
    rawmv [OPTION]... -t <DIRECTORY> <SOURCE>...\
",
    after_help = "\
Copyright (C) 2021 oxalica<oxalicc@pm.me>
This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 3.
This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
"
)]
struct RawOpt {
    /// Do not prompt before overwriting.
    /// Note that unlike mv(1), without this flag, we raise error if destination exists.
    #[structopt(long, short)]
    force: bool,
    /// Silently skip files whose destination exists.
    #[structopt(long, short, conflicts_with_all = &["force", "interactive"])]
    no_clobber: bool,
    /// Prompt before overwrite.
    #[structopt(long, short, conflicts_with = "force")]
    interactive: bool,
    /// Print what is being done.
    #[structopt(long, short)]
    verbose: bool,

    /// Always treat the last path (destination) as a normal file.
    /// This implies that only two operands are expected.
    #[structopt(long, short = "T")]
    no_target_directory: bool,
    /// Move all files into this directory.
    #[structopt(
        long,
        short = "t",
        name = "DIRECTORY",
        conflicts_with = "no-target-directory"
    )]
    target_directory: Option<PathBuf>,

    #[structopt(hidden = true)]
    files: Vec<PathBuf>,
}

impl App {
    fn parse_args() -> Result<Self, ClapError> {
        let mut opt = RawOpt::from_args_safe()?;
        let mut this = Self {
            force: opt.force,
            no_clobber: opt.no_clobber,
            interactive: opt.interactive,
            verbose: opt.verbose,
            operations: Vec::with_capacity(opt.files.len()),
        };

        let wrong_arg_num = |s| ClapError::with_description(s, ClapErrorKind::WrongNumberOfValues);

        if opt.no_target_directory {
            if opt.files.len() != 2 {
                return Err(wrong_arg_num("-T expects exact 2 file operands"));
            }
            let [src, dest]: [_; 2] = opt.files.try_into().unwrap();
            this.push_move_to_target(src, dest)?;
        } else if let Some(target_dir) = opt.target_directory {
            if opt.files.is_empty() {
                return Err(wrong_arg_num("Missing file operand"));
            }
            this.push_move_to_dir(opt.files, &target_dir)?;
        } else {
            match opt.files.len() {
                0 => return Err(wrong_arg_num("Missing file operand")),
                1 => return Err(wrong_arg_num("Missing destination operand")),
                2 if !opt.files[1].is_dir() => {
                    let [src, dest]: [_; 2] = opt.files.try_into().unwrap();
                    this.push_move_to_target(src, dest)?;
                }
                _ => {
                    let target_dir = opt.files.pop().unwrap();
                    this.push_move_to_dir(opt.files, &target_dir)?;
                }
            }
        }

        Ok(this)
    }

    fn push_move_to_target(&mut self, src: PathBuf, dest: PathBuf) -> Result<(), ClapError> {
        fs::symlink_metadata(&src).map_err(|_| {
            ClapError::with_description(
                &format!("File not found: {:?}", src),
                ClapErrorKind::InvalidValue,
            )
        })?;
        self.operations.push((src, dest));
        Ok(())
    }

    fn push_move_to_dir(
        &mut self,
        srcs: impl IntoIterator<Item = PathBuf>,
        target_dir: &Path,
    ) -> Result<(), ClapError> {
        if !target_dir.is_dir() {
            return Err(ClapError::with_description(
                &format!("Not a directory: {:?}", target_dir),
                ClapErrorKind::InvalidValue,
            ));
        }
        for src in srcs {
            let name = src.file_name().ok_or_else(|| {
                ClapError::with_description(
                    &format!("Path doesn't have base name: {:?}", src),
                    ClapErrorKind::InvalidValue,
                )
            })?;
            let dest = target_dir.join(name);
            self.push_move_to_target(src, dest)?;
        }
        Ok(())
    }
}

fn main() {
    let app = App::parse_args().unwrap_or_else(|err| ClapError::exit(&err));

    let mut failed = false;
    for (src, dest) in &app.operations {
        let mut ret = do_rename(src, dest, app.force);
        if !app.force && matches!(&ret, Err(err) if err.kind() == io::ErrorKind::AlreadyExists) {
            if app.no_clobber {
                continue;
            } else if app.interactive {
                eprint!("Overwrite {:?} -> {:?} ? [y/N] ", src, dest);
                let _ = io::stderr().flush();
                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);
                if input.trim() == "y" {
                    ret = do_rename(src, dest, true);
                } else {
                    continue;
                }
            }
        }

        match ret {
            Ok(()) => {
                if app.verbose {
                    eprintln!("Renamed {:?} -> {:?}", src, dest);
                }
            }
            Err(err) => {
                eprintln!("Cannot rename {:?} -> {:?}: {}", src, dest, err);
                failed = true;
            }
        }
    }

    if failed {
        process::exit(1);
    }
}

#[cfg(unix)]
fn do_rename(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let src_c = CString::new(src.as_os_str().as_bytes()).unwrap();
    let dest_c = CString::new(dest.as_os_str().as_bytes()).unwrap();
    let flag = if overwrite { 0 } else { libc::RENAME_NOREPLACE };
    let ret = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            src_c.as_ptr(),
            libc::AT_FDCWD,
            dest_c.as_ptr(),
            flag,
        )
    };
    if ret == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
