use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Node {
    name: std::ops::Range<usize>,
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
    names: Vec<u8>,
    nodes: Vec<Node>,
    errors: Vec<String>,
}

fn store_name(names: &mut Vec<u8>, name: &OsStr) -> std::ops::Range<usize> {
    let start = names.len();
    names.extend_from_slice(name.as_bytes());
    start..names.len()
}

impl Node {
    fn name<'a>(&self, names: &'a [u8]) -> &'a OsStr {
        OsStr::from_bytes(&names[self.name.clone()])
    }
}

fn add_size(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b).ok_or_else(|| "size overflow".into())
}

fn node_path(nodes: &[Node], names: &[u8], mut id: usize) -> PathBuf {
    let mut components = Vec::new();
    while id != 0 {
        components.push(nodes[id].name(names));
        id = nodes[id].parent;
    }
    let mut path = PathBuf::from(nodes[0].name(names));
    for name in components.into_iter().rev() {
        path.push(name);
    }
    path
}

fn finish_scan(
    nodes: &mut [Node],
    names: &[u8],
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
            let loser = if node_path(nodes, names, id).as_os_str().as_bytes()
                < node_path(nodes, names, *winner).as_os_str().as_bytes()
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

fn next_name<'a>(remaining: &mut &'a [u8]) -> io::Result<&'a CStr> {
    if remaining.len() < 20 {
        return Err(io::Error::other("truncated directory record"));
    }
    let length = u16::from_ne_bytes([remaining[16], remaining[17]]) as usize;
    if length < 20 || length > remaining.len() {
        return Err(io::Error::other("invalid directory record length"));
    }
    let name = CStr::from_bytes_until_nul(&remaining[19..length])
        .map_err(|_| io::Error::other("unterminated directory name"))?;
    *remaining = &remaining[length..];
    Ok(name)
}

// Linux dirent64 has a 19-byte header followed by a NUL-terminated name.
// Parse bytes instead of casting to a potentially unaligned C structure.
fn visit_directory(
    path: &Path,
    buffer: &mut [u8],
    mut visit: impl FnMut(&OsStr, io::Result<libc::stat>),
) -> io::Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: path is NUL-terminated. The returned FD is owned below.
    let raw = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open returned a new descriptor; this guard closes it on all exits.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    loop {
        // SAFETY: fd is live and buffer points to writable storage of this length.
        let count = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                fd.as_raw_fd(),
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Ok(());
        }
        let bytes = buffer
            .get(..count as usize)
            .ok_or_else(|| io::Error::other("invalid directory buffer length"))?;
        let mut remaining = bytes;
        while !remaining.is_empty() {
            let name = next_name(&mut remaining)?;
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fd and the bounded NUL-terminated name are live, and stat is writable.
            let result = unsafe {
                libc::fstatat(
                    fd.as_raw_fd(),
                    name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            let metadata = if result == 0 {
                // SAFETY: successful fstatat initialized the stat fields.
                Ok(unsafe { stat.assume_init() })
            } else {
                Err(io::Error::last_os_error())
            };
            visit(OsStr::from_bytes(name.to_bytes()), metadata);
        }
    }
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
    let mut names = Vec::new();
    let nodes = vec![Node {
        name: store_name(&mut names, root.as_os_str()),
        allocated: metadata.blocks().checked_mul(512).ok_or("size overflow")?,
        apparent: metadata.len(),
        parent: 0,
        directory: true,
    }];
    let directories = vec![Some(Directory {
        node: 0,
        parent: None,
        pending_children: 0,
    })];
    let hard_links = Vec::new();
    let errors = Vec::new();
    // The counter includes queued and running directories, so idle workers can
    // distinguish temporary starvation from completion without polling.
    let queue = Mutex::new((vec![(root, 0usize)], 1usize));
    let wake = Condvar::new();
    let next_directory = AtomicUsize::new(1);
    // ponytail: directory merges serialize; shard storage if this limits measured scaling.
    let collected = Mutex::new((nodes, names, directories, hard_links, errors));
    let results = std::thread::scope(|scope| {
        let worker = || {
            let mut nodes = Vec::new();
            let mut names = Vec::new();
            let mut directories = Vec::new();
            let mut links = Vec::new();
            let mut errors = Vec::new();
            let mut overflow = false;
            let flush = |nodes: &mut Vec<Node>,
                         names: &mut Vec<u8>,
                         directories: &mut Vec<(usize, Directory)>,
                         links: &mut Vec<(usize, (u64, u64))>,
                         errors: &mut Vec<String>| {
                let mut collected = collected.lock().unwrap();
                let (all_nodes, all_names, all_dirs, all_links, all_errors) = &mut *collected;
                let offset = all_nodes.len();
                for (id, mut directory) in directories.drain(..) {
                    directory.node += offset;
                    all_dirs.resize_with(all_dirs.len().max(id + 1), || None);
                    all_dirs[id] = Some(directory);
                }
                all_links.extend(
                    links
                        .drain(..)
                        .map(|(id, identity)| (id + offset, identity)),
                );
                let name_offset = all_names.len();
                for node in nodes.iter_mut() {
                    node.name.start += name_offset;
                    node.name.end += name_offset;
                }
                all_names.append(names);
                all_nodes.append(nodes);
                all_errors.append(errors);
            };
            let mut local = None;
            let mut buffer = [0u8; 32768];
            loop {
                let (path, parent) = if let Some(job) = local.take() {
                    job
                } else {
                    let mut state = queue.lock().unwrap();
                    loop {
                        if let Some(job) = state.0.pop() {
                            break job;
                        }
                        if state.1 == 0 {
                            flush(
                                &mut nodes,
                                &mut names,
                                &mut directories,
                                &mut links,
                                &mut errors,
                            );
                            return overflow;
                        }
                        state = wake.wait(state).unwrap();
                    }
                };
                let mut children = Vec::new();
                let result = visit_directory(&path, &mut buffer, |name, metadata| {
                    let metadata = match metadata {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            errors.push(format!("{:?}: {error}", path.join(name)));
                            return;
                        }
                    };
                    let id = nodes.len();
                    let allocated = match u64::try_from(metadata.st_blocks)
                        .ok()
                        .and_then(|n| n.checked_mul(512))
                    {
                        Some(size) => size,
                        None => {
                            overflow = true;
                            0
                        }
                    };
                    let apparent = match u64::try_from(metadata.st_size) {
                        Ok(size) => size,
                        Err(_) => {
                            overflow = true;
                            0
                        }
                    };
                    let is_dir = metadata.st_mode & libc::S_IFMT == libc::S_IFDIR;
                    nodes.push(Node {
                        name: store_name(&mut names, name),
                        allocated,
                        apparent,
                        parent,
                        directory: is_dir,
                    });
                    if is_dir {
                        let directory = next_directory.fetch_add(1, Ordering::Relaxed);
                        directories.push((
                            directory,
                            Directory {
                                node: id,
                                parent: Some(parent),
                                pending_children: 0,
                            },
                        ));
                        if !one_filesystem || metadata.st_dev == device {
                            children.push((path.join(name), directory));
                            // Publish discovered subdirectories before a large parent
                            // finishes, so other workers can start immediately.
                            if children.len() == 16 {
                                let mut state = queue.lock().unwrap();
                                state.1 += children.len();
                                state.0.append(&mut children);
                                wake.notify_all();
                            }
                        }
                    } else if metadata.st_nlink > 1 {
                        links.push((id, (metadata.st_dev, metadata.st_ino)));
                    }
                });
                if let Err(error) = result {
                    errors.push(format!("{path:?}: {error}"));
                }
                if nodes.len() >= 256 {
                    flush(
                        &mut nodes,
                        &mut names,
                        &mut directories,
                        &mut links,
                        &mut errors,
                    );
                }
                let mut state = queue.lock().unwrap();
                state.1 += children.len();
                state.1 -= 1;
                local = children.pop();
                let notify = !children.is_empty() || state.1 == 0;
                state.0.extend(children);
                if notify {
                    wake.notify_all();
                }
            }
        };
        let handles: Vec<_> = (1..threads).map(|_| scope.spawn(worker)).collect();
        let mut results = vec![worker()];
        results.extend(handles.into_iter().map(|h| h.join().unwrap()));
        results
    });
    if results.into_iter().any(|overflow| overflow) {
        return Err("size overflow".into());
    }
    let (mut nodes, names, mut directories, hard_links, mut errors) =
        collected.into_inner().unwrap();
    finish_scan(&mut nodes, &names, &mut directories, hard_links)?;
    errors.sort();
    Ok(Scan {
        names,
        nodes,
        errors,
    })
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
        threads: 0,
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
    if options.threads == 0 {
        options.threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
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
        children.sort_unstable_by(|a, b| {
            size(b)
                .cmp(&size(a))
                .then(a.name(&result.names).cmp(b.name(&result.names)))
        });
        for node in children {
            writeln!(out, "{}\t{:?}", size(node), node.name(&result.names))
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
        for directory in 0..40 {
            let path = root.join(format!("parallel-{directory}"));
            fs::create_dir(&path).unwrap();
            for file in 0..10 {
                fs::write(path.join(file.to_string()), b"parallel").unwrap();
            }
        }
        assert!(
            visit_directory(&root.join("cycle"), &mut [0u8; 32768], |_, _| panic!(
                "followed symlink"
            ))
            .is_err()
        );
        let long_names = root.join("long-names");
        fs::create_dir(&long_names).unwrap();
        for id in 0..140 {
            fs::write(
                long_names.join(format!("{id:03}{}", "x".repeat(252))),
                b"long",
            )
            .unwrap();
        }
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
            let path = node_path(&serial.nodes, &serial.names, id);
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
                            node_path(&s.nodes, &s.names, id),
                            (
                                n.allocated,
                                n.apparent,
                                node_path(&s.nodes, &s.names, n.parent),
                            ),
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
        let mut names = Vec::new();
        let mut make = |name: &str, size, parent, directory| Node {
            name: store_name(&mut names, OsStr::new(name)),
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
        finish_scan(&mut nodes, &names, &mut directories, Vec::new()).unwrap();
        assert_eq!(
            nodes.iter().map(|n| n.allocated).collect::<Vec<_>>(),
            [15, 8, 12, 14]
        );
        assert_eq!(
            node_path(&nodes, &names, 1),
            Path::new("/root/parent/child/file")
        );
    }

    #[test]
    fn validates_directory_records() {
        let mut record = [0u8; 24];
        record[16..18].copy_from_slice(&24u16.to_ne_bytes());
        record[19..23].copy_from_slice(b"a\xffb\0");
        let mut bytes = record.as_slice();
        assert_eq!(next_name(&mut bytes).unwrap().to_bytes(), b"a\xffb");
        assert!(bytes.is_empty());
        assert!(next_name(&mut &record[..19]).is_err());
        record[16..18].copy_from_slice(&0u16.to_ne_bytes());
        assert!(next_name(&mut record.as_slice()).is_err());
        record[16..18].copy_from_slice(&25u16.to_ne_bytes());
        assert!(next_name(&mut record.as_slice()).is_err());
        record[16..18].copy_from_slice(&24u16.to_ne_bytes());
        record[19..].fill(b'x');
        assert!(next_name(&mut record.as_slice()).is_err());
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
