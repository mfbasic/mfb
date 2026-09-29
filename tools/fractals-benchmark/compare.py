#!/usr/bin/env python3
"""Build temporary, deterministic GUI runs of both fractals examples."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"Expected exactly one instrumentation anchor: {old!r}")
    return source.replace(old, new, 1)


def instrument(source, output, frames):
    source = replace_once(source, 'IMPORT io', 'IMPORT io\nIMPORT fs')
    source = replace_once(source, '  MUT size AS canvas::Size = canvas::getSize()', '''  LET settleUntil AS Integer = datetime::monotonicNanos() + 2000000000
  DO WHILE datetime::monotonicNanos() < settleUntil
    canvas::present([empty])
    canvas::waitFrame()
  LOOP
  MUT size AS canvas::Size = canvas::getSize()''')
    source = replace_once(source, '  LET started AS Integer = datetime::monotonicNanos()',
                          '  MUT report AS String = ""\n  MUT frame AS Integer = 0')
    source = replace_once(source, '  DO WHILE NOT quitPressed()', f'''  DO WHILE frame < {frames * 15 + 3}
    LET frameStart AS Integer = datetime::monotonicNanos()
    MUT checksum AS Integer = 0''')
    source = replace_once(source, '    LET elapsed AS Integer = datetime::monotonicNanos() - started', f'''    LET sampleIndex AS Integer = math::max(0, frame - 3) / {frames}
    LET position AS Integer = sampleIndex MOD 3
    MUT localTime AS Integer = 0
    IF position = 1 THEN localTime = 6000000000
    IF position = 2 THEN localTime = 11000000000
    LET elapsed AS Integer = (sampleIndex / 3) * VIEW_NANOS + localTime''')
    # Identical checksum work for both representations, after crossfade blending.
    anchor = '      LET dot AS canvas::DrawItem' if '      LET dot AS canvas::DrawItem' in source else '      LET offset AS Integer'
    source = replace_once(source, anchor, '      checksum = checksum + (i + 1) * (shown.r + shown.g * 256 + shown.b * 65536)\n' + anchor)
    source = replace_once(source, '    canvas::present(scene)\n    canvas::waitFrame()', '''    canvas::present(scene)
    LET submitted AS Integer = datetime::monotonicNanos()
    canvas::waitFrame()
    LET finished AS Integer = datetime::monotonicNanos()
    IF frame >= 3 THEN
      report = report & toString(sampleIndex) & "," & toString(submitted - frameStart) & "," & toString(finished - frameStart) & "," & toString(checksum) & "," & toString(size.width) & "," & toString(size.height) & "\\n"
    END IF
    frame = frame + 1''')
    return replace_once(source, '  RETURN 0\nEND FUNC', f'  fs::writeText("{output}", report)\n  RETURN 0\nEND FUNC')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mfb', type=Path, default=ROOT / 'target/release/mfb')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--frames', type=int, default=6, help='frames per fixed position')
    parser.add_argument('--output', type=Path, default=Path('/tmp/fractals-benchmark.json'))
    args = parser.parse_args()
    if args.rounds < 1 or args.frames < 1:
        parser.error('rounds and frames must be positive')
    sources = {name: (ROOT / 'examples' / name / 'src/main.mfb').read_text()
               for name in ('fractals', 'fractals-image')}
    # The math, palette, and crossfade functions must remain identical.
    common = lambda s: s[s.index('FUNC smoothstep'):s.index('FUNC main AS Integer')]
    assert common(sources['fractals']) == common(sources['fractals-image'])
    result = {'rounds': args.rounds, 'frames_per_position': args.frames, 'runs': []}
    env = os.environ.copy()
    env.pop('MFB_MACAPP_HEADLESS', None)  # Real Metal rendering, not headless mode.
    with tempfile.TemporaryDirectory(prefix='mfb-fractals-bench-') as tmp:
        base = Path(tmp)
        for name, source in sources.items():
            project = base / name
            (project / 'src').mkdir(parents=True)
            manifest = json.loads((ROOT / 'examples' / name / 'project.json').read_text())
            (project / 'project.json').write_text(json.dumps(manifest))
            (project / 'src/main.mfb').write_text(instrument(source, project / 'frames.csv', args.frames))
            subprocess.run([str(args.mfb.resolve()), 'build', str(project)], check=True, cwd=ROOT)
        for round_index in range(args.rounds):
            order = list(sources) if round_index % 2 == 0 else list(reversed(sources))
            for name in order:
                project = base / name
                log = project / 'frames.csv'
                log.unlink(missing_ok=True)
                bundle = project / 'build' / f'{name}.app'
                print(f'Running round {round_index + 1}: {name}', flush=True)
                subprocess.run(['open', '-n', '-W', str(bundle)], check=True, timeout=240, cwd=ROOT, env=env,
                               stdout=subprocess.DEVNULL)
                rows = [list(map(int, line.split(','))) for line in log.read_text().splitlines()]
                assert len(rows) == args.frames * 15
                result['runs'].append({'round': round_index + 1, 'name': name, 'rows': rows})
        args.output.write_text(json.dumps(result, indent=2) + '\n')
        reference = [[r[0], *r[3:]] for r in result['runs'][0]['rows']]
        for run in result['runs']:
            assert [[r[0], *r[3:]] for r in run['rows']] == reference, 'Workload/checksum/dimensions mismatch'
    for name in sources:
        rows = [row for run in result['runs'] if run['name'] == name for row in run['rows']]
        print(f'{name}: mean build+submit {statistics.mean(r[1] for r in rows)/1e6:.3f} ms; '
              f'mean paced frame {statistics.mean(r[2] for r in rows)/1e6:.3f} ms; '
              f'median paced frame {statistics.median(r[2] for r in rows)/1e6:.3f} ms')
    print(f'All colour checksums and canvas dimensions match. Raw results: {args.output}')


if __name__ == '__main__':
    main()
