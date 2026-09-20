#!/usr/bin/env python3
"""Compare full-tree rudu, ncdu, and an optional frozen baseline on warm caches."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import tempfile
import time


def output(command):
    return subprocess.check_output(command, text=True).strip()


def fixture(base, name, count, width):
    root = base / name
    marker = base / (name + '.ready')
    if marker.exists():
        if json.loads(marker.read_text()) != [count, width]:
            raise RuntimeError(f'Fixture configuration changed: {root}')
        return root
    root.mkdir(parents=True, exist_ok=False)
    print(f'Generating {name}: {count:,} files', flush=True)
    for start in range(0, count, width):
        directory = root / f'd{start // width:06}'
        directory.mkdir()
        for i in range(start, min(start + width, count)):
            (directory / f'f{i:09}').write_bytes(b'x' * 128)
    marker.write_text(json.dumps([count, width]))
    return root


def totals(rudu, root):
    return {k: int(v) for k, v in (line.split('\t') for line in
            output([str(rudu), str(root), '--threads', '1']).splitlines())}


def verify(rudu, ncdu, root, scratch, export):
    result = totals(rudu, root)
    if result['errors']:
        raise RuntimeError(f'Incomplete baseline: {root}')
    expected = {
        'allocated_bytes': int(output(['du', '-s', '-B1', '--', str(root)]).split()[0]),
        # GNU du omits directory lengths in apparent-size mode; ncdu and rudu include them.
        'apparent_bytes': int(output(['du', '-s', '-B1', '--apparent-size', '--', str(root)]).split()[0])
            + sum(os.lstat(directory).st_size for directory, _, _ in os.walk(root)),
    }
    for key, value in expected.items():
        if result[key] != value:
            raise RuntimeError(f'{root}: rudu {key}={result[key]}, du={value}')
    if export:
        # Synthetic fixtures have no hard links; summing ncdu's own-size fields is unambiguous.
        target = scratch / 'ncdu-check.json'
        subprocess.run([str(ncdu), '--ignore-config', '-0', '-t', '1', '-o', str(target), str(root)], check=True)
        tree = json.loads(target.read_text())[3]
        stack = [tree]
        counted = dict(allocated_bytes=0, apparent_bytes=0, entries=0)
        while stack:
            item = stack.pop()
            if isinstance(item, list):
                stack.extend(item[1:])
                item = item[0]
            if item.get('read_error') or item.get('excluded'):
                raise RuntimeError(f'ncdu returned incomplete fixture: {root}')
            counted['entries'] += 1
            counted['allocated_bytes'] += item.get('dsize', 0)
            counted['apparent_bytes'] += item.get('asize', 0)
        for key, value in counted.items():
            if result[key] != value:
                raise RuntimeError(f'{root}: rudu {key}={result[key]}, ncdu={value}')
        target.unlink()
        expected['ncdu_export_matches'] = True
    for threads in [4, 8]:
        parallel = {k: int(v) for k, v in (line.split('\t') for line in
                    output([str(rudu), str(root), '--threads', str(threads)]).splitlines())}
        if parallel != result:
            raise RuntimeError(f'Parallel totals differ: {root}')
    return {'rudu': result, 'reference': expected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ncdu', type=Path, required=True)
    parser.add_argument('--rudu', type=Path, default=Path('target/release/rudu'))
    parser.add_argument('--baseline', type=Path, help='Optional frozen rudu binary for paired comparisons')
    parser.add_argument('--data-root', type=Path, default=Path('.bench/data'))
    parser.add_argument('--output', type=Path, default=Path('docs/benchmarks/latest.json'))
    parser.add_argument('--real', type=Path, help='Optional existing tree; scanned read-only')
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error('--runs must be positive')
    rudu, ncdu = args.rudu.resolve(), args.ncdu.resolve()
    base = args.data_root.resolve()
    base.mkdir(parents=True, exist_ok=True)
    cases = [(fixture(base, 'spread-100k', 100_000, 100), True),
             (fixture(base, 'wide-100k', 100_000, 100_000), True),
             (fixture(base, 'spread-1m', 1_000_000, 1000), True)]
    if args.real:
        cases.append((args.real.resolve(), False))
    report = {
        'timestamp_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'platform': platform.platform(), 'cpu_count': os.cpu_count(),
        'cpu': output(['lscpu', '-J']),
        'storage': output(['findmnt', '-T', str(base), '-o', 'SOURCE,FSTYPE,TARGET']),
        'devices': output(['lsblk', '-d', '-o', 'NAME,MODEL,ROTA']),
        'rustc': output(['rustc', '--version']),
        'rudu': output([str(rudu), '--version']),
        'ncdu': output([str(ncdu), '--version']),
        'rudu_sha256': hashlib.sha256(rudu.read_bytes()).hexdigest(),
        'ncdu_sha256': hashlib.sha256(ncdu.read_bytes()).hexdigest(),
        'source_sha256': hashlib.sha256(Path('src/main.rs').read_bytes()).hexdigest(),
        'cargo_lock_sha256': hashlib.sha256(Path('Cargo.lock').read_bytes()).hexdigest(),
        'build': 'cargo build --release (default profile)',
        'cache': 'warm: verification plus one warm-up per configuration; no cache drops',
        'runs': args.runs, 'order_seed': 20260921,
        'scope': 'process launch through full scan, accounting, and process teardown; tree retained by both tools',
        'cases': [],
        'baseline_sha256': hashlib.sha256(args.baseline.read_bytes()).hexdigest() if args.baseline else None,
        'baseline_source_sha256': hashlib.sha256(Path('benchmarks/baseline/src/main.rs').read_bytes()).hexdigest() if args.baseline else None,
    }
    rng = random.Random(report['order_seed'])
    with tempfile.TemporaryDirectory(prefix='rudu-benchmark-') as temporary:
        scratch = Path(temporary)
        timing_file = scratch / 'time.txt'
        for root, synthetic in cases:
            print(f'Checking {root.name}', flush=True)
            verified = verify(rudu, ncdu, root, scratch, synthetic)
            configs = []
            for threads in [1, 4, 8]:
                configs.extend([
                    ('rudu', threads, [str(rudu), str(root), '--threads', str(threads), '--scan-only']),
                    ('ncdu', threads, [str(ncdu), '--ignore-config', '-0', '--quit-after-scan', '-t', str(threads), str(root)]),
                ])
            if args.baseline:
                baseline = args.baseline.resolve()
                if totals(baseline, root) != verified['rudu']:
                    raise RuntimeError(f'Baseline totals differ: {root}')
                configs.extend(('baseline', n, [str(baseline), str(root), '--threads', str(n), '--scan-only'])
                               for n in [1, 4, 8])
            measurements = {(tool, threads): [] for tool, threads, _ in configs}
            for _, _, command in configs:
                subprocess.run(command, stdout=subprocess.DEVNULL, check=True)
            for repetition in range(args.runs):
                order = configs.copy()
                rng.shuffle(order)
                for tool, threads, command in order:
                    start = time.perf_counter()
                    completed = subprocess.run(['/usr/bin/time', '-f', '%M %U %S', '-o', str(timing_file), *command],
                                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
                    elapsed = time.perf_counter() - start
                    if completed.returncode:
                        raise RuntimeError(f'{tool} failed: {completed.stderr}')
                    rss, user, system = timing_file.read_text().split()
                    measurements[tool, threads].append(dict(seconds=elapsed, peak_rss_kib=int(rss),
                                                           user_seconds=float(user), system_seconds=float(system)))
                print(f'  {root.name}: round {repetition + 1}/{args.runs}', flush=True)
            # Detect changes across the measurement window before accepting real-tree evidence.
            if totals(rudu, root) != verified['rudu']:
                raise RuntimeError(f'Tree changed during benchmark: {root}')
            case = {'path': str(root), 'synthetic': synthetic, 'verification': verified, 'results': []}
            for tool, threads, command in configs:
                runs = measurements[tool, threads]
                times = [r['seconds'] for r in runs]
                case['results'].append(dict(tool=tool, threads=threads, command=command, runs=runs,
                    median_seconds=statistics.median(times), min_seconds=min(times), max_seconds=max(times),
                    median_peak_rss_kib=statistics.median(r['peak_rss_kib'] for r in runs)))
            report['cases'].append(case)
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(f'Results: {args.output}', flush=True)


if __name__ == '__main__':
    main()
