use std::convert::TryInto;
use std::path::{Path, PathBuf};
use std::{fs, process};
use structopt::clap::{Error as ClapError, ErrorKind as ClapErrorKind};
use structopt::StructOpt;

#[derive(Debug)]
struct App {
    force: bool,
    verbose: bool,
    operations: Vec<(PathBuf, PathBuf)>,
}

/// mv but without cp fallback.
#[derive(StructOpt, Debug)]
#[structopt(usage = "\
    rawmv [OPTION]... [-T] <SOURCE> <DEST>
    rawmv [OPTION]... <SOURCE>... <DIRECTORY>
    rawmv [OPTION]... -t <DIRECTORY> <SOURCE>...\
")]
struct RawOpt {
    /// Do not prompt before overwriting.
    #[structopt(long, short)]
    force: bool,
    /// Print what is being done.
    #[structopt(long, short)]
    verbose: bool,

    /// Always treat the last path (destination) as a normal file.
    /// This implies that only two operands are expected.
    #[structopt(long, short = "T")]
    no_target_directory: bool,
    /// Move all files into this directory.
    #[structopt(long, short = "t", name = "DIRECTORY", conflicts_with = "no-target-directory")]
    target_directory: Option<PathBuf>,

    #[structopt(hidden = true)]
    files: Vec<PathBuf>,
}

impl App {
    fn parse_args() -> Result<Self, ClapError> {
        let mut opt = RawOpt::from_args_safe()?;
        let mut this = Self {
            force: opt.force,
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

    if !app.force {
        eprintln!("Rename without --force is not implemented yet");
        process::exit(1);
    }

    let mut failed = false;
    for (src, dest) in &app.operations {
        match fs::rename(src, dest) {
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
