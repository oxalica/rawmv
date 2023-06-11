// SPDX-License-Identifier: GPL-3.0-only
#![warn(clippy::pedantic)]
use std::convert::TryInto;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{anyhow, bail, ensure, Result};
use pico_args::Arguments;

// We truly want boolean productions, not one-at-a-time.
// See: https://github.com/rust-lang/rust-clippy/issues/10923
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct App {
    force: bool,
    no_clobber: bool,
    interactive: bool,
    verbose: bool,
    operations: Vec<(PathBuf, PathBuf)>,
}

impl App {
    fn help() -> String {
        format!(
            "\
rawmv {version}
mv(1) but without cp(1) fallback. Simple wrapper of renameat2(2).

USAGE:
    rawmv [OPTION]... [-T] <SOURCE> <DEST>
    rawmv [OPTION]... <SOURCE>... <DIRECTORY>
    rawmv [OPTION]... -t <DIRECTORY> <SOURCE>...

FLAGS:
    -f, --force                 Do not prompt before overwriting. Note that
                                unlike mv(1), without this flag, we raise an
                                error if the destination already exists
    -h, --help                  Prints help informatio.
    -i, --interactive           Prompt for confirmation before overwrite
    -n, --no-clobber            Silently skip files whose destinations exist
    -T, --no-target-directory   Always treat the last path (destination) as a
                                normal file. This implies that only two
                                operands are expected
    -V, --version               Prints version information
    -v, --verbose               Print what is being done

OPTIONS:
    -t, --target-directory <DIRECTORY>  Move all files into this directory

Copyright (C) 2021-2022 Oxalica <oxalicc@pm.me>
This program is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, version 3.
This program is distributed in the hope that it will be useful, but WITHOUT
ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
",
            version = env!("CARGO_PKG_VERSION")
        )
    }

    fn parse_env() -> Result<Self> {
        Self::parse_args(std::env::args_os().skip(1))
    }

    fn parse_args<I: IntoIterator<Item = S>, S: Into<OsString>>(args: I) -> Result<Self> {
        let mut raw_args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
        let tail_positionals = match raw_args.iter().position(|s| s == "--") {
            None => Vec::new(),
            Some(pos) => {
                let tail = raw_args.drain(pos + 1..).collect();
                raw_args.pop();
                tail
            }
        };

        let mut args = Arguments::from_vec(raw_args);

        if args.contains(["-h", "--help"]) {
            print!("{}", Self::help());
            process::exit(0);
        }

        if args.contains(["-V", "--version"]) {
            println!("rawmv {}", env!("CARGO_PKG_VERSION"));
            process::exit(0);
        }

        let mut this = Self {
            force: args.contains(["-f", "--force"]),
            no_clobber: args.contains(["-n", "--no-clobber"]),
            interactive: args.contains(["-i", "--interactive"]),
            verbose: args.contains(["-v", "--verbose"]),
            operations: Vec::new(),
        };
        let target_directory = args
            .opt_value_from_os_str::<_, PathBuf, String>(["-t", "--target-directory"], |s| {
                Ok(s.to_os_string().into())
            })?;
        let no_target_directory = args.contains(["-T", "--no-target-directory"]);

        ensure!(
            !this.force || !this.no_clobber,
            "Cannot use '--force' and '--no-clobber' together"
        );
        ensure!(
            target_directory.is_none() || !no_target_directory,
            "Cannot use '--no-target-directory' and '--target-directory' together"
        );

        let mut positionals = args
            .finish()
            .into_iter()
            .chain(tail_positionals)
            .map(Into::into)
            .collect::<Vec<PathBuf>>();

        if no_target_directory {
            let [src, dest]: [_; 2] = positionals.try_into().map_err(|_| {
                anyhow!("Expect exact 2 operands when using '--no-target-directory'")
            })?;
            this.operations.push((src, dest));
        } else if let Some(target_dir) = target_directory {
            ensure!(!positionals.is_empty(), "Missing file operand");
            this.push_move_to_dir(positionals, &target_dir)?;
        } else {
            match positionals.len() {
                0 => bail!("Missing file operand"),
                1 => bail!("Missing destination operand"),
                2 if !positionals[1].is_dir() => {
                    let [src, dest]: [_; 2] = positionals.try_into().unwrap();
                    this.operations.push((src, dest));
                }
                _ => {
                    let target_dir = positionals.pop().unwrap();
                    this.push_move_to_dir(positionals, &target_dir)?;
                }
            }
        }

        Ok(this)
    }

    fn push_move_to_dir(
        &mut self,
        srcs: impl IntoIterator<Item = PathBuf>,
        target_dir: &Path,
    ) -> Result<()> {
        for src in srcs {
            let base = src
                .file_name()
                .ok_or_else(|| anyhow!("Source doesn't have base name: {}", src.display()))?;
            let dest = target_dir.join(base);
            self.operations.push((src, dest));
        }
        Ok(())
    }
}

fn main() {
    let app = App::parse_env().unwrap_or_else(|err| {
        eprintln!("rawmv: {err}");
        process::exit(1);
    });

    let mut failed = false;
    for (src, dest) in &app.operations {
        let mut ret = do_rename(src, dest, app.force);
        if !app.force && matches!(&ret, Err(err) if err.kind() == io::ErrorKind::AlreadyExists) {
            if app.no_clobber {
                continue;
            } else if app.interactive {
                eprint!("rawmv: Overwrite {src:?} -> {dest:?} ? [y/N] ");
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
                    eprintln!("rawmv: Renamed {src:?} -> {dest:?}");
                }
            }
            Err(err) => {
                eprintln!("rawmv: Cannot rename {src:?} -> {dest:?}: {err}");
                failed = true;
            }
        }
    }

    if failed {
        process::exit(1);
    }
}

fn do_rename(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    use rustix::fs;

    let flags = if overwrite {
        fs::RenameFlags::empty()
    } else {
        fs::RenameFlags::NOREPLACE
    };
    fs::renameat_with(fs::cwd(), src, fs::cwd(), dest, flags)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::App;

    fn parse(args: &[&str]) -> Result<App, String> {
        App::parse_args(args.iter()).map_err(|e| e.to_string())
    }

    #[test]
    fn test_parse_auto_detect() {
        assert_eq!(
            parse(&["/non/existing/file", "/"]).unwrap(),
            App {
                operations: vec![("/non/existing/file".into(), "/file".into())],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["/non/existing/file", "/non/existing/other"]).unwrap(),
            App {
                operations: vec![("/non/existing/file".into(), "/non/existing/other".into())],
                ..App::default()
            }
        );

        assert_eq!(
            parse(&["/foo", "/bar", "/non/existing"]).unwrap(),
            App {
                operations: vec![
                    ("/foo".into(), "/non/existing/foo".into()),
                    ("/bar".into(), "/non/existing/bar".into())
                ],
                ..App::default()
            }
        );

        assert_eq!(parse(&[]).unwrap_err(), "Missing file operand",);
        assert_eq!(parse(&["foo"]).unwrap_err(), "Missing destination operand",);
    }

    #[test]
    fn test_parse_no_target_dir() {
        assert_eq!(
            parse(&["-T", "/", "/"]).unwrap(),
            App {
                operations: vec![("/".into(), "/".into())],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["-T", "/"]).unwrap_err(),
            "Expect exact 2 operands when using '--no-target-directory'",
        );
        assert_eq!(
            parse(&["-T", "/", "/", "/"]).unwrap_err(),
            "Expect exact 2 operands when using '--no-target-directory'",
        );
    }

    #[test]
    fn test_parse_target_dir() {
        assert_eq!(parse(&["-t", "/"]).unwrap_err(), "Missing file operand",);
        assert_eq!(
            parse(&["-T", "-t", "/"]).unwrap_err(),
            "Cannot use '--no-target-directory' and '--target-directory' together",
        );
        assert_eq!(
            parse(&["foo", "-t"]).unwrap_err(),
            "the '-t' option doesn't have an associated value"
        );

        assert_eq!(
            parse(&["/some/non/existing/file", "-t", "/"]).unwrap(),
            App {
                operations: vec![("/some/non/existing/file".into(), "/file".into())],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["-t", "foo", "bar", "baz"]).unwrap(),
            App {
                operations: vec![
                    ("bar".into(), "foo/bar".into()),
                    ("baz".into(), "foo/baz".into())
                ],
                ..App::default()
            }
        );
    }

    #[test]
    fn test_parse_clobber_flags() {
        let app = App {
            operations: vec![("foo".into(), "/foo".into())],
            ..App::default()
        };

        assert_eq!(
            parse(&["-n", "foo", "/"]).unwrap(),
            App {
                no_clobber: true,
                ..app.clone()
            },
        );
        assert_eq!(
            parse(&["-f", "foo", "/"]).unwrap(),
            App { force: true, ..app },
        );

        assert_eq!(
            parse(&["-f", "foo", "/", "-n"]).unwrap_err(),
            "Cannot use '--force' and '--no-clobber' together"
        );
    }

    #[test]
    fn test_parse_dash_dash() {
        assert_eq!(
            parse(&["-f", "foo", "--", "-n", "-t"]).unwrap(),
            App {
                force: true,
                operations: vec![
                    ("foo".into(), "-t/foo".into()),
                    ("-n".into(), "-t/-n".into()),
                ],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["-T", "--", "--", "-f"]).unwrap(),
            App {
                operations: vec![("--".into(), "-f".into()),],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["-t", "foo", "--", "-f"]).unwrap(),
            App {
                operations: vec![("-f".into(), "foo/-f".into()),],
                ..App::default()
            }
        );
        assert_eq!(
            parse(&["-t", "--", "-f"]).unwrap_err(),
            "the '-t' option doesn't have an associated value"
        );
    }
}
