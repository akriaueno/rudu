use dua_core::{Options as WalkOptions, Order, walk};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Node {
    name: OsString,
    allocated: u64,
    apparent: u64,
    // A directory ID during collection, then a node index after resolution.
    parent: usize,
    directory: bool,
}

struct Directory {
    node: usize,
    parent: Option<usize>,
    pending_children: usize,
}

struct Scan {
    nodes: Vec<Node>,
    errors: Vec<String>,
}

fn add_size(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b).ok_or_else(|| "size overflow".into())
}

fn node_path(nodes: &[Node], mut id: usize) -> PathBuf {
    let mut components = Vec::new();
    while id != 0 {
        components.push(&nodes[id].name);
        id = nodes[id].parent;
    }
    let mut path = PathBuf::from(&nodes[0].name);
    for name in components.into_iter().rev() {
        path.push(name);
    }
    path
}

fn finish_scan(
    nodes: &mut [Node],
    directories: &mut [Option<Directory>],
    hard_links: Vec<(usize, (u64, u64))>,
) -> Result<(), String> {
    for node in nodes.iter_mut() {
        node.parent = directories
            .get(node.parent)
            .and_then(Option::as_ref)
            .ok_or("walker returned a child without its parent")?
            .node;
    }
    let mut identities = HashMap::new();
    // Only duplicate hard links need path comparisons; normal entries never reconstruct paths.
    for (id, identity) in hard_links {
        let winner = identities.entry(identity).or_insert(id);
        if *winner != id {
            let loser = if node_path(nodes, id).as_os_str().as_bytes()
                < node_path(nodes, *winner).as_os_str().as_bytes()
            {
                std::mem::replace(winner, id)
            } else {
                id
            };
            nodes[loser].allocated = 0;
            nodes[loser].apparent = 0;
        }
    }
    // Files contribute once. Directories wait until every child directory has contributed.
    for id in 1..nodes.len() {
        if !nodes[id].directory {
            let parent = nodes[id].parent;
            nodes[parent].allocated = add_size(nodes[parent].allocated, nodes[id].allocated)?;
            nodes[parent].apparent = add_size(nodes[parent].apparent, nodes[id].apparent)?;
        }
    }
    for id in 0..directories.len() {
        if let Some(parent) = directories[id].as_ref().and_then(|d| d.parent) {
            let parent = directories
                .get_mut(parent)
                .and_then(Option::as_mut)
                .ok_or("walker returned a directory without its parent")?;
            parent.pending_children = parent
                .pending_children
                .checked_add(1)
                .ok_or("directory count overflow")?;
        }
    }
    let mut ready: Vec<_> = directories
        .iter()
        .enumerate()
        .filter_map(|(id, d)| d.as_ref().filter(|d| d.pending_children == 0).map(|_| id))
        .collect();
    let mut completed = 0;
    while let Some(id) = ready.pop() {
        let directory = directories[id].as_ref().unwrap();
        let node = directory.node;
        let parent = directory.parent;
        completed += 1;
        if let Some(parent_id) = parent {
            let parent = directories[parent_id].as_mut().unwrap();
            nodes[parent.node].allocated =
                add_size(nodes[parent.node].allocated, nodes[node].allocated)?;
            nodes[parent.node].apparent =
                add_size(nodes[parent.node].apparent, nodes[node].apparent)?;
            parent.pending_children -= 1;
            if parent.pending_children == 0 {
                ready.push(parent_id);
            }
        }
    }
    if completed != directories.iter().flatten().count() {
        return Err("walker returned a directory cycle".into());
    }
    Ok(())
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
    let device = metadata.dev();
    let mut nodes: Vec<Node> = Vec::new();
    let mut directories = Vec::new();
    let mut hard_links = Vec::new();
    let mut errors = Vec::new();
    let entries = walk(
        &root,
        threads,
        Order::Completion,
        WalkOptions::default(),
        move |entry| {
            entry.metadata.as_ref().is_some_and(|m| {
                m.as_ref()
                    .is_ok_and(|m| !one_filesystem || m.dev() == device)
            })
        },
    );
    for entry in entries {
        let mut entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("while scanning {root:?}: {error}"));
                continue;
            }
        };
        let metadata = match entry.metadata.take() {
            Some(Ok(metadata)) => metadata,
            Some(Err(error)) => {
                errors.push(format!("{:?}: {error}", entry.path()));
                continue;
            }
            None => return Err("walker returned an entry without metadata".into()),
        };
        let id = nodes.len();
        let parent = match entry.parent_directory_id {
            Some(parent) => Some(parent.index()),
            None if id == 0 && metadata.is_dir() => None,
            None => return Err("root changed during scanning".into()),
        };
        if let Some(directory) = entry.directory_id {
            directories.resize_with(directories.len().max(directory.index() + 1), || None);
            directories[directory.index()] = Some(Directory {
                node: id,
                parent,
                pending_children: 0,
            });
        }
        nodes.push(Node {
            name: if id == 0 {
                root.clone().into_os_string()
            } else {
                entry.file_name
            },
            allocated: metadata.blocks().checked_mul(512).ok_or("size overflow")?,
            apparent: metadata.len(),
            parent: parent.unwrap_or(0),
            directory: metadata.is_dir(),
        });
        if !metadata.is_dir() && metadata.nlink() > 1 {
            hard_links.push((id, (metadata.dev(), metadata.ino())));
        }
    }
    if nodes.is_empty() {
        return Err("root disappeared during scanning".into());
    }
    finish_scan(&mut nodes, &mut directories, hard_links)?;
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
                        "Usage: rudu [PATH] [--threads N] [-x] [--apparent-size] [--list] [--scan-only]\n\nExact parallel Linux scanner. Includes hidden files; does not follow symlinks.\nPrints tab-separated totals in bytes. --list adds immediate children.\nExit codes: 0 complete, 1 fatal error, 2 partial scan."
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
            .skip(1)
            .filter(|n| n.parent == 0)
            .collect();
        let size = |n: &Node| {
            if options.apparent {
                n.apparent
            } else {
                n.allocated
            }
        };
        children.sort_unstable_by(|a, b| size(b).cmp(&size(a)).then(a.name.cmp(&b.name)));
        for node in children {
            writeln!(out, "{}\t{:?}", size(node), &node.name).map_err(|e| e.to_string())?;
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
    use std::collections::HashSet;
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
        fs::hard_link(root.join("a/file"), root.join("a!first")).unwrap();
        fs::create_dir_all(root.join("deep/child/grandchild")).unwrap();
        fs::write(root.join("deep/child/grandchild/file"), b"nested").unwrap();
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
        for (id, node) in serial.nodes.iter().enumerate().skip(1) {
            assert!(node.parent < serial.nodes.len());
            let path = node_path(&serial.nodes, id);
            if path == root.join("a/file") || path == root.join("z/linked") {
                assert_eq!((node.allocated, node.apparent), (0, 0));
            }
            if path == root.join("a!first") {
                assert_eq!(node.apparent, 5);
            }
        }
        for threads in [2, 4, 8] {
            let parallel = scan(&root, threads, false).unwrap();
            assert!(parallel.errors.is_empty());
            let values = |s: &Scan| {
                s.nodes
                    .iter()
                    .enumerate()
                    .map(|(id, n)| {
                        (
                            node_path(&s.nodes, id),
                            (n.allocated, n.apparent, node_path(&s.nodes, n.parent)),
                        )
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
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
    fn aggregates_children_received_before_their_directories() {
        let make = |name: &str, size, parent, directory| Node {
            name: name.into(),
            allocated: size,
            apparent: size,
            parent,
            directory,
        };
        // Arrival order: root, grandchild file, child directory, parent directory.
        // Directory IDs are root=0, parent=1, child=2; node IDs deliberately differ.
        let mut nodes = vec![
            make("/root", 1, 0, true),
            make("file", 8, 2, false),
            make("child", 4, 1, true),
            make("parent", 2, 0, true),
        ];
        let mut directories = vec![
            Some(Directory {
                node: 0,
                parent: None,
                pending_children: 0,
            }),
            Some(Directory {
                node: 3,
                parent: Some(0),
                pending_children: 0,
            }),
            Some(Directory {
                node: 2,
                parent: Some(1),
                pending_children: 0,
            }),
        ];
        finish_scan(&mut nodes, &mut directories, Vec::new()).unwrap();
        assert_eq!(
            nodes.iter().map(|n| n.allocated).collect::<Vec<_>>(),
            [15, 8, 12, 14]
        );
        assert_eq!(node_path(&nodes, 1), Path::new("/root/parent/child/file"));
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
