#!/usr/bin/env python3
"""Check ratio direction, equivalence, and rejection of unpaired input."""
from benchmark import compare_paired

reference = [dict(seconds=1), dict(seconds=2), dict(seconds=3)]
assert compare_paired(reference, reference)['equivalent_within_five_percent']
faster = [dict(seconds=r['seconds'] / 2) for r in reference]
result = compare_paired(faster, reference)
assert result['faster_at_95_percent'] and not result['equivalent_within_five_percent']
assert abs(result['geometric_mean_ratio'] - 0.5) < 1e-12
assert not compare_paired(reference, faster)['within_five_percent_slowdown']
try:
    compare_paired(reference, faster[:2])
except ValueError:
    pass
else:
    raise AssertionError('Unpaired input accepted')
print('Benchmark statistics checks passed')
