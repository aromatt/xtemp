use clap::Parser;
use std::process;
use std::fmt;
use std::process::Stdio;
use shell_escape::escape;
use std::io::{self, BufRead, BufReader, Write, Seek, SeekFrom};
use std::process::Command;
use tempfile::NamedTempFile;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Number of lines per batch. xtemp will write batch_size lines to batch_size tempfiles,
    /// and pass those tempfiles as arguments to the command.
    #[arg(short = 'n', long)]
    batch_size: Option<usize>,

    /// Replacement string for tempfile arguments. If not specified, tempfiles are appended as trailing arguments.
    #[arg(short = 'J', long)]
    replstr: Option<String>,

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

fn run(args: Args) -> Result<()> {
    if args.command.is_empty() {
        return Err(XtempError::MissingCommand);
    }

    let batch_size = args.batch_size.unwrap_or(1);

    run_batch_mode(batch_size, args.replstr.as_deref(), &args.command)
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

fn run_batch_mode(batch_size: usize, replstr: Option<&str>, command: &[String]) -> Result<()> {
    let stdin = io::stdin();
    let lines: Vec<String> = stdin.lock().lines()
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| XtempError::InvalidUtf8(e))?;

    let mut temp_pool: Vec<NamedTempFile> = (0..batch_size)
        .map(|_| NamedTempFile::new().map_err(|e| XtempError::FailedToWrite(e)))
        .collect::<Result<_>>()?;

    for chunk in lines.chunks(batch_size) {
        let mut file_paths = Vec::new();

        // Reuse temp files from the pool
        for (i, line) in chunk.iter().enumerate() {
            let tmpfile = &mut temp_pool[i];
            let file = tmpfile.as_file_mut();

            // TODO DRY
            file.set_len(0).map_err(|e| XtempError::FailedToWrite(e))?;
            file.seek(SeekFrom::Start(0)).map_err(|e| XtempError::FailedToWrite(e))?;
            writeln!(file, "{}", line).map_err(|e| XtempError::FailedToWrite(e))?;
            file.flush().map_err(|e| XtempError::FailedToWrite(e))?;
            file_paths.push(tmpfile.path().to_path_buf());
        }

        // Build command with file arguments
        let mut cmd_args = Vec::new();

        match replstr {
            Some(replstr) => {
                // Replace exact matches of replstr with space-separated temp file paths
                for arg in command {
                    if arg == replstr {
                        let files_str = file_paths
                            .iter()
                            .map(|p| escape(p.to_string_lossy()))
                            .collect::<Vec<_>>()
                            .join(" ");
                        cmd_args.push(files_str);
                    } else {
                        cmd_args.push(arg.clone());
                    }
                }
            }
            None => {
                // Append file paths as trailing arguments
                cmd_args.extend(command.iter().cloned());
                for path in &file_paths {
                    cmd_args.push(escape(path.to_string_lossy()).to_string());
                }
            }
        }

        let mut child = Command::new(&cmd_args[0])
            .args(&cmd_args[1..])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| XtempError::SubprocessFailed(e.to_string()))?;

        let stdout = child.stdout.take().ok_or_else(|| {
            XtempError::SubprocessFailed("subprocess has no stdout".into())
        })?;

        let reader = BufReader::new(stdout);
        for (_, line) in chunk.iter().zip(reader.lines()) {
            let line = line.map_err(|e| XtempError::InvalidUtf8(e))?;
            println!("{}", line);
        }

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
