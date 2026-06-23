use decibel_hotindex_core::{CfChecksum, HotIndexError, Result};
#[cfg(feature = "rocksdb")]
use decibel_hotindex_storage::RocksDbEngine;
#[cfg(feature = "rocksdb")]
use decibel_hotindex_storage::StorageEngine;
use std::env;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    match command {
        "checksum" => checksum_command(&args[1..]),
        "compare-checksum" => compare_checksum_command(&args[1..]),
        "help" | "--help" | "-h" => {
            print_usage();
            Ok(())
        }
        other => Err(HotIndexError::Config(format!("unknown command: {other}"))),
    }
}

fn checksum_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let engine = opts.optional_value("--engine").unwrap_or("rocksdb");
    let checksums = match engine {
        "rocksdb" => rocksdb_checksums(&opts)?,
        other => {
            return Err(HotIndexError::Config(format!(
                "unsupported checksum engine: {other}"
            )));
        }
    };

    if let Some(out) = opts.optional_value("--out") {
        write_checksums(Path::new(out), &checksums)?;
    } else {
        serde_json::to_writer_pretty(std::io::stdout(), &checksums).map_err(json_error)?;
        println!();
    }
    Ok(())
}

#[cfg(feature = "rocksdb")]
fn rocksdb_checksums(opts: &Args<'_>) -> Result<Vec<CfChecksum>> {
    let db_path = opts.required_path("--db-path")?;
    RocksDbEngine::open(db_path)?.checksums()
}

#[cfg(not(feature = "rocksdb"))]
fn rocksdb_checksums(_opts: &Args<'_>) -> Result<Vec<CfChecksum>> {
    Err(HotIndexError::Config(
        "RocksDB checksum requires `--features rocksdb`".to_string(),
    ))
}

fn compare_checksum_command(args: &[String]) -> Result<()> {
    let opts = Args::new(args);
    let left = read_checksums(&opts.required_path("--left")?)?;
    let right = read_checksums(&opts.required_path("--right")?)?;

    if left != right {
        return Err(HotIndexError::Storage(format!(
            "checksum mismatch\nleft={}\nright={}",
            serde_json::to_string_pretty(&left).map_err(json_error)?,
            serde_json::to_string_pretty(&right).map_err(json_error)?
        )));
    }

    println!("checksum comparison passed: {} logical CFs", left.len());
    Ok(())
}

fn write_checksums(path: &Path, checksums: &[CfChecksum]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    serde_json::to_writer_pretty(BufWriter::new(file), checksums).map_err(json_error)
}

fn read_checksums(path: &Path) -> Result<Vec<CfChecksum>> {
    let file = File::open(path)?;
    serde_json::from_reader(BufReader::new(file)).map_err(json_error)
}

fn json_error(error: serde_json::Error) -> HotIndexError {
    HotIndexError::Parse(error.to_string())
}

struct Args<'a> {
    args: &'a [String],
}

impl<'a> Args<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args }
    }

    fn required_path(&self, name: &str) -> Result<PathBuf> {
        self.optional_value(name)
            .map(PathBuf::from)
            .ok_or_else(|| HotIndexError::Config(format!("missing required argument: {name}")))
    }

    fn optional_value(&self, name: &str) -> Option<&'a str> {
        self.args
            .windows(2)
            .find(|window| window[0] == name)
            .map(|window| window[1].as_str())
    }
}

fn print_usage() {
    eprintln!(
        "usage:
  decibel-admin checksum --engine rocksdb --db-path <path> [--out <checksums.json>]
  decibel-admin compare-checksum --left <checksums.json> --right <checksums.json>"
    );
}
