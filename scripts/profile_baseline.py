#!/usr/bin/env python3
"""Build isolated diagnostic variants; leave src/main.rs and the baseline binary unchanged."""
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
WORK = ROOT / '.bench/profile'
OUTPUT = ROOT / 'docs/benchmarks/profile-latest.json'


def replace(source, old, new):
    assert source.count(old) == 1, (old, source.count(old))
    return source.replace(old, new)


def instrument(source):
    source = replace(source, '    let metadata = fs::symlink_metadata(root)',
                     '    let phase_start = std::time::Instant::now();\n    let metadata = fs::symlink_metadata(root)')
    source = replace(source, '    let (mut nodes, mut errors) = records.into_inner().unwrap();',
                     '    let collected_at = std::time::Instant::now();\n    let (mut nodes, mut errors) = records.into_inner().unwrap();')
    source = replace(source, '    if nodes.first()',
                     '    let sorted_at = std::time::Instant::now();\n    if nodes.first()')
    source = replace(source, '    // Path sorting places',
                     '    let linked_at = std::time::Instant::now();\n    // Path sorting places')
    source = replace(source, '    errors.sort();\n    Ok(Scan { nodes, errors })', '''    errors.sort();
    drop(directories);
    drop(identities);
    let aggregated_at = std::time::Instant::now();
    eprintln!("PROFILE\\tcollect\\t{}", (collected_at - phase_start).as_secs_f64());
    eprintln!("PROFILE\\tsort\\t{}", (sorted_at - collected_at).as_secs_f64());
    eprintln!("PROFILE\\tparent_link\\t{}", (linked_at - sorted_at).as_secs_f64());
    eprintln!("PROFILE\\taggregate\\t{}", (aggregated_at - linked_at).as_secs_f64());
    eprintln!("PROFILE\\tnode_bytes\\t{}", std::mem::size_of::<Node>());
    eprintln!("PROFILE\\tnode_capacity\\t{}", nodes.capacity());
    Ok(Scan { nodes, errors })''')
    source = replace(source, '    Ok(if result.errors.is_empty() { 0 } else { 2 })', '''    let exit_code = if result.errors.is_empty() { 0 } else { 2 };
    let drop_start = std::time::Instant::now();
    drop(result);
    eprintln!("PROFILE\\tdrop\\t{}", drop_start.elapsed().as_secs_f64());
    Ok(exit_code)''')
    return source


def local_collection(source):
    source = replace(source, 'struct Scan {', '''struct LocalRecords<'a> {
    shared: &'a Mutex<(Vec<Node>, Vec<String>)>,
    records: (Vec<Node>, Vec<String>),
}
impl Drop for LocalRecords<'_> {
    fn drop(&mut self) {
        let mut shared = self.shared.lock().unwrap();
        shared.0.append(&mut self.records.0);
        shared.1.append(&mut self.records.1);
    }
}

struct Scan {''')
    source = replace(source, '        .run(|| {\n            Box::new(|entry| {', '''        .run(|| {
            let mut local = LocalRecords { shared: &records, records: (Vec::new(), Vec::new()) };
            Box::new(move |entry| {''')
    return replace(source, '                let mut records = records.lock().unwrap();',
                   '                let records = &mut local.records;')


def relative_storage(source):
    source = replace(source, '    WalkBuilder::new(&root)', '    let scan_root = &root;\n    WalkBuilder::new(&root)')
    source = replace(source, 'path: path.to_path_buf(),', 'path: path.strip_prefix(scan_root).unwrap().to_path_buf(),')
    return replace(source, '!= Some(&root)', '!= Some(&PathBuf::new())')


def main():
    os.chdir(ROOT)
    snapshot = ROOT / 'benchmarks/baseline'
    original = (snapshot / 'src/main.rs').read_text()
    source_hash = hashlib.sha256(original.encode()).hexdigest()
    baseline = ROOT / '.bench/baseline/rudu'
    if not baseline.exists():
        baseline.parent.mkdir(parents=True, exist_ok=True)
        build_env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / '.bench/baseline-target'))
        subprocess.run(['cargo', 'build', '--release', '--locked', '--manifest-path', str(snapshot / 'Cargo.toml')], env=build_env, check=True)
        shutil.copyfile(ROOT / '.bench/baseline-target/release/rudu', baseline)
        baseline.chmod(0o755)
    binary_hash = hashlib.sha256(baseline.read_bytes()).hexdigest()
    (WORK / 'src/bin').mkdir(parents=True, exist_ok=True)
    for name in ['Cargo.toml', 'Cargo.lock']:
        shutil.copyfile(snapshot / name, WORK / name)
    variants = {
        'phase_baseline': instrument(original),
        'worker_local': instrument(local_collection(original)),
        'relative_paths': instrument(relative_storage(original)),
        'local_relative': instrument(relative_storage(local_collection(original))),
    }
    for name, source in variants.items():
        (WORK / f'src/bin/{name}.rs').write_text(source)
    env = dict(os.environ, CARGO_TARGET_DIR=str(WORK / 'target'))
    for operation in [['test'], ['build', '--release']]:
        subprocess.run(['cargo', *operation, '--locked', '--offline', '--manifest-path', str(WORK / 'Cargo.toml')], env=env, check=True)
    dataset = ROOT / '.bench/data/spread-1m'
    configs = [('original', n, [str(baseline), str(dataset), '--threads', str(n)]) for n in [1, 8]]
    ncdu = ROOT / '.bench/tools/ncdu'
    configs += [('ncdu', n, [str(ncdu), '--ignore-config', '-0', '--quit-after-scan', '-t', str(n), str(dataset)]) for n in [1, 8]]
    configs += [(name, n, [str(WORK / f'target/release/{name}'), str(dataset), '--threads', str(n)])
                for name in variants for n in ([1, 8] if name == 'phase_baseline' else [8])]
    expected = subprocess.check_output(configs[0][2], text=True)
    for name, _, command in configs:
        result = subprocess.run(command, capture_output=True, text=True, check=True)
        if name != 'ncdu':
            assert result.stdout == expected, name
    records = {(name, n): [] for name, n, _ in configs}
    rng = random.Random(20260921)
    timing_file = WORK / 'time.txt'
    for repetition in range(5):
        order = configs.copy()
        rng.shuffle(order)
        for name, n, command in order:
            start = time.perf_counter()
            result = subprocess.run(['/usr/bin/time', '-f', '%M %U %S', '-o', str(timing_file), *command],
                                    capture_output=True, text=True, check=True)
            elapsed = time.perf_counter() - start
            if name != 'ncdu':
                assert result.stdout == expected, name
            phases = {}
            for line in result.stderr.splitlines():
                if line.startswith('PROFILE\t'):
                    _, key, value = line.split('\t')
                    phases[key] = float(value)
            rss, user, system = timing_file.read_text().split()
            records[name, n].append(dict(seconds=elapsed, peak_rss_kib=int(rss), user_seconds=float(user),
                                         system_seconds=float(system), phases=phases))
        print(f'Profiling round {repetition + 1}/5', flush=True)
    report = {
        'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'source_sha256': source_hash, 'baseline_binary_sha256': binary_hash,
        'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'dataset': str(dataset), 'cache': 'warm', 'runs': 5, 'order_seed': 20260921,
        'perf': 'unavailable: perf_event_paranoid=4; no host settings changed',
        'results': [],
    }
    for name, n, command in configs:
        runs = records[name, n]
        report['results'].append(dict(name=name, threads=n, command=command, runs=runs,
            median_seconds=statistics.median(r['seconds'] for r in runs),
            median_peak_rss_kib=statistics.median(r['peak_rss_kib'] for r in runs),
            phase_medians={key: statistics.median(r['phases'][key] for r in runs) for key in runs[0]['phases']}))
    assert hashlib.sha256((snapshot / 'src/main.rs').read_bytes()).hexdigest() == source_hash
    assert hashlib.sha256(baseline.read_bytes()).hexdigest() == binary_hash
    OUTPUT.write_text(json.dumps(report, indent=2) + '\n')
    print(f'Results: {OUTPUT}', flush=True)


if __name__ == '__main__':
    main()
