"""Witness each re-scoped transition independently of animation and timing."""
import copy
import json
from pathlib import Path
import sys

EVENTS = [(60, 7, 'split', 0, (0, 1)), (150, 7, 'zoom', 0, (0,)),
          (240, 6, 'selection-ends-zoom', 0, (0, 1)), (330, 7, 'zoom-selected', 0, (1,)),
          (420, 7, 'restore', 0, (0, 1)), (510, 8, 'popup-show', 0, (0, 1)),
          (600, 8, 'popup-hide', 0, (0, 1)), (690, 9, 'tab-switch', 1, (0, 1)),
          (780, 9, 'tab-return', 0, (0, 1))]
FULL = [8., 50., 1904., 986.]
SPLIT = [[8., 50., 948., 986.], [964., 50., 948., 986.]]


def snapshots(frames):
    names = {}
    result = []
    alive = {}
    for frame in frames:
        names.update({m['id']: m['nm'] for m in frame.get('m', [])})
        alive.update({e[0]: e for e in frame.get('e', [])})
        for gone in frame.get('g', []):
            alive.pop(gone, None)
        result.append({names[i]: e for i, e in alive.items()})
    return result


def channel_snapshots(frames):
    names, values, result = {}, {}, []
    for frame in frames:
        names.update({m['id']: m['name'] for m in frame.get('cm', [])})
        values.update(dict(frame.get('c', [])))
        for gone in frame.get('cg', []):
            values.pop(gone, None)
        result.append({names[i]: v for i, v in values.items()})
    return result


def check(frames):
    assert len(frames) == 930
    views, channels = snapshots(frames), channel_snapshots(frames)
    witnessed = []
    roots = lambda v: {n: p for n, p in v.items() if n.count('/') == 2 and ('shell-' in n or n.endswith('/popup'))}
    for k, key, label, tab, panes in EVENTS:
        assert frames[k]['t'] == round(k * 1000 / 120, 3), (label, 'clock')
        if frames[k].get('ev'):
            assert f'key Function({key})' in frames[k]['ev'], (label, 'wrong input')
        before, after = views[k - 1], views[k]
        assert roots(before) != roots(after), (label, 'no visible transition')
        expected = {f'#0/#1/tab-{tab}-shell-{p}': SPLIT[p] if len(panes) == 2 else FULL for p in panes}
        if label == 'popup-show':
            expected['#0/#1/popup'] = FULL
        end = next((event[0] for event in EVENTS if event[0] > k), len(frames))
        for j in range(k, end):
            current = roots(views[j])
            assert set(current) == set(expected), (label, j, 'wrong mounted roots')
            for name, rect in expected.items():
                row = current[name]
                assert all(abs(a - b) < .01 for a, b in zip(row[1:5], rect)), (label, j, name, 'rectangle')
                visible = row[8:12] if len(row) >= 12 else row[1:5]
                assert all(abs(a - b) < .01 for a, b in zip(visible, rect)), (label, j, name, 'clipped')
                assert channels[j].get(name + '.scale') == 1, (label, j, 'scale')
                assert channels[j].get(name + '.blur') == 0, (label, j, 'blur')
                if label in ('popup-show', 'popup-hide') and 'shell-' in name:
                    assert row[0] == before[name][0], (label, j, 'underlying pane remounted')
            assert all(abs(e[5] - 1) < 1e-6 for e in views[j].values()), (label, j, 'unexpected fade')
            # poll observes runtime focus on the following frame after a focus request.
            if j >= k + 2:
                focus = 'focus-none' if label == 'popup-show' else 'focus-pane-' + ('0' if label in ('split', 'zoom') else '1')
                assert '#0/#2/' + focus in views[j], (label, j, 'runtime focus not restored')
        witnessed.append({'label': label, 'frame': k, 'event_cpu_ms': frames[k]['quality']['cpu_ms'],
                          'window_max_ms': max(f['quality']['cpu_ms'] for f in frames[k:end]),
                          'window_frames': end - k, 'roots': expected})
    return witnessed


def negative_checks(frames):
    views = snapshots(frames)
    for k, _, label, _, _ in EVENTS:
        for mutation in ('no-op', 'fade', 'late-collapse', 'clip'):
            bad = copy.deepcopy(frames)
            if mutation == 'no-op':
                bad[k]['e'] = list(views[k - 1].values())
                bad[k]['g'] = [e[0] for e in frames[k].get('e', []) if e[0] not in {p[0] for p in views[k - 1].values()}]
            else:
                j = k if mutation == 'fade' else k + 5
                row = next(copy.deepcopy(e) for n, e in views[j].items() if n.count('/') == 2 and 'shell-' in n)
                if mutation == 'fade':
                    row[5] = .5
                elif mutation == 'late-collapse':
                    row[3] = 1
                else:
                    row = row[:8] + [row[1], row[2], 1., 1.]
                bad[j]['e'] = [e for e in bad[j].get('e', []) if e[0] != row[0]] + [row]
            try:
                check(bad)
            except AssertionError as error:
                assert error.args[0][0] == label, (label, mutation, 'wrong failure', error)
            else:
                raise AssertionError((label, mutation, 'bad transition accepted'))


if __name__ == '__main__':
    rows = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
    assert rows[0]['clock'] == 'virtual' and rows[0]['hz'] == 120 and rows[0]['scale'] == 2
    result = check(rows[1:])
    negative_checks(rows[1:])
    print(json.dumps(result, indent=2))
