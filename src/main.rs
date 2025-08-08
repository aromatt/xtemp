use clap::Parser;
use std::process;
use std::fmt;
use std::process::Stdio;
use shell_escape::escape;
use std::io::{self, BufRead, Write, Seek, SeekFrom};
use std::process::Command;
use tempfile::NamedTempFile;
use nix::sys::resource::{getrlimit, Resource};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about,
    help_template = "\
{before-help}{name} {version}
{author-with-newline}
{about-with-newline}
{usage-heading} {usage}\n
{all-args}{after-help}
")]
struct Args {
    /// Number of lines per batch (size of tempfile pool)
    #[arg(short = 'n', long)]
    batch_size: Option<usize>,

    /// Replacement string for tempfile arguments (if not specified, tempfiles are appended as trailing arguments)
    #[arg(short = 'J', long)]
    replstr: Option<String>,

    /// Keep newlines when writing lines to tempfiles (default: strip newlines)
    #[arg(long)]
    keep_newlines: bool,

    /// Instead of passing all tempfiles as arguments, pass a single file containing a list of the
    /// tempfile paths
    #[arg(short = 'l', long)]
    list: bool,

    /// Command to execute with tempfile arguments
    command: Vec<String>,
}

#[derive(Debug)]
pub enum XtempError {
    InvalidUtf8(std::io::Error),
    FailedToWrite(std::io::Error),
    SubprocessFailed(String),
    MissingCommand,
}

impl fmt::Display for XtempError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use XtempError::*;
        match self {
            InvalidUtf8(e) => write!(f, "input contains invalid UTF-8: {}", e),
            FailedToWrite(e) => write!(f, "could not write to output stream: {}", e),
            SubprocessFailed(msg) => write!(f, "subprocess failed: {}", msg),
            MissingCommand => write!(f, "missing command argument"),
        }
    }
}

pub type Result<T> = std::result::Result<T, XtempError>;

fn get_max_open_files() -> usize {
    match getrlimit(Resource::RLIMIT_NOFILE) {
        Ok((soft, _hard)) => soft as usize,
        Err(_) => 1024, // fallback
    }
}

/// Replaces replstr with replacements, returning the full literal command.
fn resolve_replstr(
    command: &[String],
    replstr: Option<&str>,
    replacements: Vec<String>,
) -> Vec<String> {
    let mut cmd_args = Vec::new();
    match replstr {
        Some(replstr) => {
            // Replace exact matches of replstr with all replacements
            for arg in command {
                if arg == replstr {
                    cmd_args.extend(replacements.clone());
                } else {
                    cmd_args.push(arg.clone());
                }
            }
        }
        None => {
            // Append all replacements as trailing arguments
            cmd_args.extend(command.iter().cloned());
            cmd_args.extend(replacements);
        }
    }
    cmd_args
}

fn main() {
    let args = Args::parse();
    let result = run(args);

    match result {
        Ok(_) => {}
        Err(e) => {
            eprintln!("xtemp: {}", e);
            process::exit(1);
        }
    }

}

fn run(args: Args) -> Result<()> {
    if args.command.is_empty() {
        return Err(XtempError::MissingCommand);
    }

    let batch_size = args.batch_size.unwrap_or_else(|| {
        // Default to a reasonable batch size based on open file limits, leaving some room for
        // standard streams and other files
        get_max_open_files().saturating_sub(32)
    });

    let stdin = io::stdin();
    let lines: Vec<String> = stdin.lock().lines()
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| XtempError::InvalidUtf8(e))?;

    // Create tempfile pool
    let mut pool: Vec<NamedTempFile> = (0..batch_size)
        .map(|_| NamedTempFile::new().map_err(|e| XtempError::FailedToWrite(e)))
        .collect::<Result<_>>()?;

    // Maybe create list file
    let mut list = if args.list {
        Some(NamedTempFile::new().map_err(|e| XtempError::FailedToWrite(e))?)
    } else {
        None
    };

    for chunk in lines.chunks(batch_size) {
        let mut file_paths = Vec::new();

        // Reuse temp files from the pool
        for (i, line) in chunk.iter().enumerate() {
            let tmpfile = &mut pool[i];
            let file = tmpfile.as_file_mut();

            // TODO DRY
            file.set_len(0).map_err(|e| XtempError::FailedToWrite(e))?;
            file.seek(SeekFrom::Start(0)).map_err(|e| XtempError::FailedToWrite(e))?;
            if args.keep_newlines {
                writeln!(file, "{}", line).map_err(|e| XtempError::FailedToWrite(e))?;
            } else {
                write!(file, "{}", line).map_err(|e| XtempError::FailedToWrite(e))?;
            }
            file.flush().map_err(|e| XtempError::FailedToWrite(e))?;
            file_paths.push(tmpfile.path().to_path_buf());
        }

        // Build command with file arguments
        let tempfile_args = match list {
            Some(ref mut list_tmpfile) => {
                // Write temp file paths to the list file
                let file = list_tmpfile.as_file_mut();
                file.set_len(0).map_err(|e| XtempError::FailedToWrite(e))?;
                file.seek(SeekFrom::Start(0)).map_err(|e| XtempError::FailedToWrite(e))?;
                for path in &file_paths {
                    writeln!(file, "{}", path.display())
                        .map_err(|e| XtempError::FailedToWrite(e))?;
                    }
                file.flush().map_err(|e| XtempError::FailedToWrite(e))?;
                vec![escape(list_tmpfile.path().to_string_lossy()).to_string()]
            }
            None => {
                // Pass temp files directly
                file_paths
                    .iter()
                    .map(|p| escape(p.to_string_lossy()).to_string())
                    .collect()
            }
        };

        let full_cmd = resolve_replstr(&args.command, args.replstr.as_deref(), tempfile_args);

        let mut child = Command::new(&full_cmd[0])
            .args(&full_cmd[1..])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| XtempError::SubprocessFailed(e.to_string()))?;

        let status = child.wait().map_err(|_| {
            XtempError::SubprocessFailed("failed to wait for command".into())
        })?;

        if !status.success() {
            return Err(XtempError::SubprocessFailed(format!(
                "command exited with code {}",
                status.code().unwrap_or(-1)
            )));
        }
    }
    Ok(())
}
