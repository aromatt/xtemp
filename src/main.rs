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
    /// Coprocess shell command. By default, the coprocess receives a copy of xtemp's stdin, and is
    /// expected to generate one line of output for each input line.
    #[arg(short = 'c')]
    coproc: String,

    /// Number of lines per batch. In streaming mode (default), xtemp will send batch_size lines
    /// to stdin of each coprocess followed by an EOF, and then restart each coprocess. In arg
    /// mode, xtemp execute each coprocess passing batch_size lines as arguments at a time. In
    /// file-arg mode, xtemp will write batch_size lines to batch_size tempfiles, and pass those
    /// tempfiles as arguments to each coprocess.
    #[arg(short = 'n', long)]
    batch_size: Option<usize>,

    /// Replacement string for tempfile arguments in file-arg mode.
    #[arg(short = 'F', long)]
    batch_replstr: Option<String>,

    /// Replacement string for arguments in arg mode.
    #[arg(short = 'J', long)]
    replstr: Option<String>,

}

#[derive(Debug)]
pub enum XtempError {
    InvalidBatchMode,
    InvalidUtf8(std::io::Error),
    FailedToWrite(std::io::Error),
    SubprocessFailed(String),
    MissingArgs(&'static str),
}

impl fmt::Display for XtempError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use XtempError::*;
        match self {
            InvalidBatchMode => write!(f, "invalid batch mode: both -n and -F are required for batch mode"),
            InvalidUtf8(e) => write!(f, "input contains invalid UTF-8: {}", e),
            FailedToWrite(e) => write!(f, "could not write to output stream: {}", e),
            SubprocessFailed(msg) => write!(f, "subprocess failed: {}", msg),
            MissingArgs(arg) => write!(f, "missing required argument: {}", arg),
        }
    }
}

pub type Result<T> = std::result::Result<T, XtempError>;

fn run(args: Args) -> Result<()> {
    match (&args.batch_size, &args.batch_replstr) {
        (Some(n), Some(r)) => run_batch_mode(*n, r, &args.coproc),
        (None, None)       => Ok(()),//run_streaming_mode(args),
        _ => Err(XtempError::InvalidBatchMode)
    }

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

fn run_batch_mode(batch_size: usize, batch_replstr: &str, cmd: &str) -> Result<()> {
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

        // Replace replstr with all paths (escaped)
        let files_str = file_paths
            .iter()
            .map(|p| escape(p.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(" ");

        let shell_cmd = cmd.replace(&batch_replstr, &files_str);

        let mut child = Command::new("sh")
            .arg("-eu")
            .arg("-c")
            .arg(&shell_cmd)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| XtempError::SubprocessFailed(e.to_string()))?;

        let stdout = child.stdout.take().ok_or_else(|| {
            XtempError::SubprocessFailed("subprocess has no stdout".into())
        })?;

        let reader = BufReader::new(stdout);

        // TODO in map mode, inject coprocess output into main output
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
