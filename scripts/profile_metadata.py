#!/usr/bin/env python3
"""Compare metadata lookup APIs on the existing two-level million-file fixture."""
import hashlib
import json
from pathlib import Path
import random
import statistics
import subprocess


def main():
    root = Path('.bench/data/spread-1m').resolve()
    source = Path('scripts/profile_metadata.rs')
    binary = Path('.bench/profile-metadata')
    subprocess.run(['rustc', '-O', str(source), '-o', str(binary)], check=True)
    configs = [(mode, threads) for mode in ['full', 'entry'] for threads in [1, 8]]
    rng = random.Random(20260921)
    records = {config: [] for config in configs}
    for mode, threads in configs:
        subprocess.run([str(binary), str(root), str(threads), mode], stdout=subprocess.DEVNULL, check=True)
    expected = None
    for repetition in range(5):
        order = configs.copy()
        rng.shuffle(order)
        for mode, threads in order:
            values = subprocess.check_output([str(binary), str(root), str(threads), mode], text=True).split()
            totals = [int(v) for v in values[1:]]
            if expected is None:
                expected = totals
            assert totals == expected
            records[mode, threads].append(float(values[0]))
        print(f'Metadata round {repetition + 1}/5', flush=True)
    report = {
        'dataset': str(root), 'cache': 'warm',
        'scope': 'enumeration and file metadata only; no stored tree, directory sizes, sorting, or hard-link accounting',
        'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
        'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
        'order_seed': 20260921, 'totals': expected,
        'results': [{'mode': mode, 'threads': n, 'seconds': runs, 'median_seconds': statistics.median(runs)}
                    for (mode, n), runs in records.items()],
    }
    Path('docs/benchmarks/metadata.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report['results'], indent=2))


if __name__ == '__main__':
    main()
