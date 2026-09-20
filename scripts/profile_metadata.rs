// Isolated diagnostic for the two-level synthetic fixtures, not a general scanner.
// Compile: rustc -O scripts/profile_metadata.rs -o .bench/profile-metadata
use std::{env, fs, os::unix::fs::MetadataExt, path::PathBuf, thread, time::Instant};

fn main() {
    let args: Vec<_> = env::args().collect();
    let root = PathBuf::from(&args[1]).canonicalize().unwrap();
    let threads: usize = args[2].parse().unwrap();
    let relative = match args[3].as_str() {
        "full" => false,
        "entry" => true,
        _ => panic!("mode must be full or entry"),
    };
    assert!(threads > 0);
    let mut directories: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    directories.sort();
    assert!(!directories.is_empty());
    let start = Instant::now();
    let totals = thread::scope(|scope| {
        let workers: Vec<_> = directories
            .chunks(directories.len().div_ceil(threads))
            .map(|chunk| {
                scope.spawn(move || {
                    let mut total = (0_u64, 0_u64, 0_u64);
                    for directory in chunk {
                        for entry in fs::read_dir(directory).unwrap() {
                            let entry = entry.unwrap();
                            let metadata = if relative {
                                entry.metadata().unwrap()
                            } else {
                                fs::symlink_metadata(entry.path()).unwrap()
                            };
                            total.0 += 1;
                            total.1 += metadata.len();
                            total.2 += metadata.blocks() * 512;
                        }
                    }
                    total
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .fold((0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
    });
    println!(
        "{} {} {} {}",
        start.elapsed().as_secs_f64(),
        totals.0,
        totals.1,
        totals.2
    );
}
