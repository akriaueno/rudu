use ignore::{WalkBuilder, WalkState};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug)]
struct Node {
    path: PathBuf,
    directory: bool,
    identity: Option<(u64, u64)>,
    allocated: u64,
    apparent: u64,
    parent: Option<usize>,
}

struct Scan {
    nodes: Vec<Node>,
    errors: Vec<String>,
}

fn add_size(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b).ok_or_else(|| "size overflow".into())
}

fn scan(root: &Path, threads: usize, one_filesystem: bool) -> Result<Scan, String> {
    if threads == 0 {
        return Err("thread count must be positive".into());
    }
    let metadata = fs::symlink_metadata(root).map_err(|e| format!("{root:?}: {e}"))?;
    if !metadata.is_dir() {
        return Err(format!(
            "root must be a directory, not a symlink or file: {root:?}"
        ));
    }
    fs::read_dir(root).map_err(|e| format!("{root:?}: {e}"))?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    // ponytail: full paths and a shared append lock form the baseline; replace only after profiling.
    let records = Mutex::new((Vec::new(), Vec::new()));
    WalkBuilder::new(&root)
        .standard_filters(false)
        .follow_links(false)
        .same_file_system(one_filesystem)
        .threads(threads)
        .build_parallel()
        .run(|| {
            Box::new(|entry| {
                let record = (|| {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let m = fs::symlink_metadata(path).map_err(|e| format!("{path:?}: {e}"))?;
                    let allocated = m.blocks().checked_mul(512).ok_or("size overflow")?;
                    Ok::<_, String>(Node {
                        path: path.to_path_buf(),
                        directory: m.is_dir(),
                        identity: (!m.is_dir() && m.nlink() > 1).then_some((m.dev(), m.ino())),
                        allocated,
                        apparent: m.len(),
                        parent: None,
                    })
                })();
                let mut records = records.lock().unwrap();
                match record {
                    Ok(node) => records.0.push(node),
                    Err(error) => records.1.push(error),
                }
                WalkState::Continue
            })
        });
    let (mut nodes, mut errors) = records.into_inner().unwrap();
    nodes.sort_unstable_by(|a, b| {
        a.path
            .as_os_str()
            .as_bytes()
            .cmp(b.path.as_os_str().as_bytes())
    });
    if nodes.first().map(|n| &n.path) != Some(&root) {
        return Err("root disappeared during scanning".into());
    }
    let mut directories = HashMap::new();
    let mut identities = HashSet::new();
    for (id, node) in nodes.iter_mut().enumerate() {
        if id != 0 {
            node.parent = node.path.parent().and_then(|p| directories.get(p).copied());
            if node.parent.is_none() {
                errors.push(format!("missing parent for {:?}", node.path));
            }
        }
        if node.directory {
            directories.insert(node.path.clone(), id);
        }
        if let Some(identity) = node.identity {
            if !identities.insert(identity) {
                node.allocated = 0;
                node.apparent = 0;
            }
        }
    }
    // Path sorting places each parent before its children, so reverse order aggregates bottom-up.
    for id in (1..nodes.len()).rev() {
        if let Some(parent) = nodes[id].parent {
            nodes[parent].allocated = add_size(nodes[parent].allocated, nodes[id].allocated)?;
            nodes[parent].apparent = add_size(nodes[parent].apparent, nodes[id].apparent)?;
        }
    }
    errors.sort();
    Ok(Scan { nodes, errors })
}

struct Options {
    root: PathBuf,
    threads: usize,
    one_filesystem: bool,
    apparent: bool,
    list: bool,
}

fn arguments(args: impl Iterator<Item = OsString>) -> Result<Option<Options>, String> {
    let mut options = Options {
        root: PathBuf::from("."),
        threads: std::thread::available_parallelism().map_or(1, |n| n.get().min(8)),
        one_filesystem: false,
        apparent: false,
        list: false,
    };
    let mut args = args;
    let mut positional = false;
    let mut literal = false;
    while let Some(arg) = args.next() {
        if !literal {
            match arg.to_str() {
                Some("--help" | "-h") => {
                    println!(
                        "Usage: rudu [PATH] [--threads N] [-x] [--apparent-size] [--list] [--scan-only]\n\nExact Linux baseline. Includes hidden files; does not follow symlinks.\nPrints tab-separated totals in bytes. --list adds immediate children.\nExit codes: 0 complete, 1 fatal error, 2 partial scan."
                    );
                    return Ok(None);
                }
                Some("--version" | "-V") => {
                    println!("rudu {}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                Some("--threads") => {
                    options.threads = args
                        .next()
                        .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                        .filter(|&n| n > 0)
                        .ok_or("--threads requires a positive integer")?;
                    continue;
                }
                Some("--scan-only") => continue,
                Some("--list") => {
                    options.list = true;
                    continue;
                }
                Some("--apparent-size") => {
                    options.apparent = true;
                    continue;
                }
                Some("-x") => {
                    options.one_filesystem = true;
                    continue;
                }
                Some("--") => {
                    literal = true;
                    continue;
                }
                _ if arg.as_bytes().starts_with(b"-") => {
                    return Err(format!("unknown option: {arg:?}"));
                }
                _ => {}
            }
        }
        if positional {
            return Err("expected at most one root path".into());
        }
        options.root = arg.into();
        positional = true;
    }
    Ok(Some(options))
}

fn run() -> Result<i32, String> {
    let Some(options) = arguments(std::env::args_os().skip(1))? else {
        return Ok(0);
    };
    let result = scan(&options.root, options.threads, options.one_filesystem)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let root = &result.nodes[0];
    writeln!(
        out,
        "allocated_bytes\t{}\napparent_bytes\t{}\nentries\t{}\nerrors\t{}",
        root.allocated,
        root.apparent,
        result.nodes.len(),
        result.errors.len()
    )
    .map_err(|e| e.to_string())?;
    if options.list {
        let mut children: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.parent == Some(0))
            .collect();
        let size = |n: &Node| {
            if options.apparent {
                n.apparent
            } else {
                n.allocated
            }
        };
        children.sort_unstable_by(|a, b| size(b).cmp(&size(a)).then(a.path.cmp(&b.path)));
        for node in children {
            writeln!(out, "{}\t{:?}", size(node), node.path.file_name().unwrap())
                .map_err(|e| e.to_string())?;
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    for error in &result.errors {
        eprintln!("rudu: {error:?}");
    }
    Ok(if result.errors.is_empty() { 0 } else { 2 })
}

fn main() {
    std::process::exit(match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("rudu: {error:?}");
            1
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };

    #[test]
    fn accounting_and_parallel_equivalence() {
        let root = std::env::temp_dir().join(format!(
            "rudu-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        fs::create_dir(root.join("a")).unwrap();
        fs::create_dir(root.join("z")).unwrap();
        fs::write(root.join("a/file"), b"hello").unwrap();
        fs::hard_link(root.join("a/file"), root.join("z/linked")).unwrap();
        fs::write(root.join(".hidden"), b"hidden").unwrap();
        fs::write(root.join(".gitignore"), b"ignored\n").unwrap();
        fs::write(root.join("ignored"), b"count me").unwrap();
        fs::write(
            root.join(OsString::from_vec(b"nonutf8-\xff\n\x1b".to_vec())),
            b"name",
        )
        .unwrap();
        fs::File::create(root.join("sparse"))
            .unwrap()
            .set_len(1 << 24)
            .unwrap();
        symlink(".", root.join("cycle")).unwrap();
        symlink("absent", root.join("broken")).unwrap();
        let mut paths = vec![root.clone()];
        let mut expected = (0, 0);
        let mut seen = HashSet::new();
        let mut count = 0;
        while let Some(path) = paths.pop() {
            let m = fs::symlink_metadata(&path).unwrap();
            count += 1;
            if m.is_dir() {
                paths.extend(fs::read_dir(path).unwrap().map(|e| e.unwrap().path()));
            }
            if m.is_dir() || seen.insert((m.dev(), m.ino())) {
                expected.0 += m.blocks() * 512;
                expected.1 += m.len();
            }
        }
        let serial = scan(&root, 1, false).unwrap();
        assert!(serial.errors.is_empty());
        assert_eq!(serial.nodes.len(), count);
        assert_eq!(
            (serial.nodes[0].allocated, serial.nodes[0].apparent),
            expected
        );
        for threads in [2, 4] {
            let parallel = scan(&root, threads, false).unwrap();
            assert!(parallel.errors.is_empty());
            let values = |s: &Scan| {
                s.nodes
                    .iter()
                    .map(|n| (n.path.clone(), n.allocated, n.apparent, n.parent))
                    .collect::<Vec<_>>()
            };
            assert_eq!(values(&serial), values(&parallel));
        }
        assert!(scan(&root.join("absent"), 1, false).is_err());
        assert!(scan(&root.join("cycle"), 1, false).is_err());
        assert!(scan(&root, 0, false).is_err());
        assert!(add_size(u64::MAX, 1).is_err());
        fs::create_dir(root.join("denied")).unwrap();
        fs::write(root.join("denied/file"), b"private").unwrap();
        fs::set_permissions(root.join("denied"), fs::Permissions::from_mode(0o0)).unwrap();
        let permission_result = scan(&root, 2, false).unwrap();
        let cannot_read = fs::read_dir(root.join("denied")).is_err();
        fs::set_permissions(root.join("denied"), fs::Permissions::from_mode(0o700)).unwrap();
        if cannot_read {
            assert!(!permission_result.errors.is_empty());
        } else {
            eprintln!("permission-error assertion skipped: process can read mode-000 directories");
        }
    }

    #[test]
    fn rejects_invalid_arguments() {
        for args in [
            vec!["--threads", "0"],
            vec!["--threads"],
            vec!["--wat"],
            vec!["a", "b"],
        ] {
            assert!(arguments(args.into_iter().map(OsString::from)).is_err());
        }
        let options = arguments(
            ["--threads", "4", "--", "-dir"]
                .into_iter()
                .map(OsString::from),
        )
        .unwrap()
        .unwrap();
        assert_eq!(options.root, Path::new("-dir"));
        assert_eq!(options.threads, 4);
    }
}
