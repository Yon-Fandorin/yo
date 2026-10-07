#!/usr/bin/env python3
"""Build static design witnesses. stdlib only; never runs depicted product actions."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import sys
import unicodedata
from cases import CASES

ROOT = Path(__file__).resolve().parent
GEOMETRIES = [(100, 34), (80, 30), (40, 24), (24, 18)]
PALETTES = {
    'dark': {'text': '#d8dee9', 'muted': '#a3adba', 'accent': '#8ebce8', 'success': '#87c5a5', 'warn': '#e3bd83', 'error': '#eea0a0'},
    'light': {'text': '#283444', 'muted': '#526174', 'accent': '#245582', 'success': '#256345', 'warn': '#765009', 'error': '#973c3c'},
    'mono': {},
}
SGR = re.compile(r'\x1b\[[0-9;]*m')
YO_PIN = '880467b3186ac7ace0111acd37cb1aa334c9dc4c'
PI_PIN = 'cd32f7725fdbddbaecdff5b1e68491563394e0ca'
CODEX_PIN = 'a956835d020762cb2b570053af06f643a11c0ecc'

def width(text):
    # Deliberately restricted fixture alphabet: no emoji/ZWJ/terminal-dependent A widths.
    return sum(0 if unicodedata.combining(c) else 2 if unicodedata.east_asian_width(c) in 'WF' else 1 for c in text)

def wrap(text, limit):
    # Keep words together when practical; hard-wrap a genuinely overlong token.
    out = []
    while width(text) > limit:
        used, cut, space = 0, 0, -1
        for i, c in enumerate(text):
            n = width(c)
            if used + n > limit: break
            used += n
            cut = i + 1
            if c == ' ' and used >= limit // 2: space = i
        if space >= 0:
            out.append(text[:space])
            text = text[space+1:]
        else:
            while cut < len(text) and unicodedata.combining(text[cut]): cut += 1
            out.append(text[:cut])
            text = text[cut:]
    out.append(text)
    return out

def role(text):
    if text.startswith('# '): return 'accent', text[2:], True
    if text.startswith('> '): return 'accent', text, True
    if text.startswith('!') or '[실패]' in text: return 'error', text, False
    if text.startswith('-'): return 'error', text, False
    if text.startswith('+') or '[완료]' in text: return 'success', text, False
    if '[대기]' in text or '[실행]' in text: return 'warn', text, False
    return 'text', text, False

def rows(text, w, color='text', bold=False):
    return [(part, color, bold) for part in wrap(text, w - 4)]

def frames(case, w, h):
    if case['id'] == '55-tiny' and w == 24 and h == 8:
        return [[('yo / 설계 예시', 'muted', False), ('승인', 'accent', True),
                 ('Proposed', 'warn', True), ('승인 잠금', 'error', True),
                 ('창을 늘리세요.', 'text', False), ('Enter 수락 불가', 'warn', True),
                 ('Esc 중단', 'accent', True), ('', 'text', False)]]
    head = rows('yo / 설계 예시', w, 'muted') + rows(case['title'], w, 'accent', True)
    head += rows(case['state'], w, 'warn', True)
    focus = rows(case['focus'], w, 'accent', True)
    for line in case.get('composer', []):
        focus += rows(line, w)
    footer = []
    for action in case['actions']:
        footer += rows(action, w, 'text', True)
    blocks = []
    for line in (case.get('body_narrow', case['body']) if w < 60 else case['body']):
        color, value, bold = role(line)
        blocks.append(rows(value, w, color, bold))
    # Header, stable focused item, content page, action area are never truncated.
    capacity = h - len(head) - len(focus) - len(footer) - 4
    if capacity < 1:
        raise ValueError(f"No readable body: {case['id']} {w}x{h}")
    chunks, current = [], []
    for block in blocks:
        if len(current) + len(block) > capacity and current:
            chunks.append(current)
            current = []
        while len(block) > capacity:
            chunks.append(block[:capacity])
            block = block[capacity:]
        current += block
    if current: chunks.append(current)
    total = len(chunks)
    result = []
    for i, current in enumerate(chunks):
        page = head + [('', 'text', False)] + current
        page += [('', 'text', False)] * (capacity - len(current))
        page += [(f'PgUp/Dn {i+1}/{total}', 'muted', False)]
        page += [('-' * (w-4), 'muted', False)] + focus + footer + [('', 'text', False)]
        assert len(page) == h, (case['id'], w, h, len(page))
        result.append(page)
    return result

def style(role_name, bold, theme):
    codes = ['1'] if bold else []
    if theme != 'mono':
        c = PALETTES[theme][role_name].lstrip('#')
        rgb = [int(c[n:n+2], 16) for n in (0, 2, 4)]
        codes.append('38;2;' + ';'.join(map(str, rgb)))
    return '\x1b[' + ';'.join(codes) + 'm' if codes else ''

def render(frame, w, theme):
    plain, ansi, markup = [], [], []
    for value, color, bold in frame:
        assert width(value) <= w - 4, (w, value)
        line = '  ' + value + ' ' * (w - 2 - width(value))
        plain.append(line)
        ansi.append(style(color, bold, theme) + line + '\x1b[0m')
        cls = f'{color}' + (' strong' if bold else '')
        markup.append(f'<span class="{cls}">{html.escape(line)}</span>')
    return '\n'.join(plain) + '\n', '\n'.join(ansi) + '\n', '\n'.join(markup)

def build():
    outputs = {}
    entries = []
    for case in CASES:
        variants = []
        geometries = GEOMETRIES + ([(24, 8)] if case['id'] == '55-tiny' else [])
        for w, h in geometries:
            fs = frames(case, w, h)
            plains = [render(f, w, 'mono')[0] for f in fs]
            plain_name = f"plain/{case['id']}.{w}x{h}.txt"
            outputs[plain_name] = ''.join(plains)
            for theme in (['dark', 'light', 'mono'] if w == 80 else ['dark']):
                rendered = [render(f, w, theme) for f in fs]
                ansi_name = f"ansi/{case['id']}.{w}x{h}.{theme}.ansi"
                outputs[ansi_name] = ''.join(x[1] for x in rendered)
                variants.append(dict(width=w, height=h, theme=theme, pages=len(fs), ansi=ansi_name,
                                     plain=plain_name, markup=[x[2] for x in rendered]))
        entries.append(dict(**case, variants=variants))
    # Static manifest has no HTML payload; source data remains compact and inspectable.
    manifest = dict(schema=1, kind='proposed-design-fixtures', runtime_verified=False,
                    pins=dict(yo=YO_PIN, pi=PI_PIN, codex=CODEX_PIN), geometries=GEOMETRIES,
                    cases=[dict(**{k:v for k,v in e.items() if k != 'variants'},
                                variants=[{k:v for k,v in x.items() if k != 'markup'} for x in e['variants']]) for e in entries],
                    files={p:hashlib.sha256(s.encode()).hexdigest() for p,s in sorted(outputs.items())})
    outputs['manifest.json'] = json.dumps(manifest, ensure_ascii=False, indent=2) + '\n'
    payload = json.dumps(entries, ensure_ascii=False).replace('<', '\\u003c').replace('>', '\\u003e').replace('&', '\\u0026')
    template = (ROOT / 'gallery.template.html').read_text()
    outputs['index.html'] = template.replace('/*__DATA__*/', payload)
    return outputs, manifest

def validate(outputs, manifest):
    covered = {t for c in CASES for t in c['coverage']}
    expected = {f'T{i:02d}' for i in range(1,65)} | {f'T22.{c}' for c in 'abcdefgh'} | {'T25.a','T29.a'}
    assert covered == expected, (expected-covered, covered-expected)
    assert len({c['id'] for c in CASES}) == len(CASES)
    assert all(c['refs'] and c['after'] for c in CASES)
    critical = {'08-working-queue': ['Esc 중단', 'Ctrl+C 중단'],
                '10-queue-edit': ['> 변경 근거와 테스트 결과를 정리해줘'],
                '15-stop-approval': ['Esc 턴 중단', 'Ctrl+C 턴 중단'],
                '17-question': ['일반 초안 보관', '빈 입력의 결과는?', '> 빈 결과 반환'],
                '20-secret': ['입력됨 · 길이 비공개'],
                '21-secret-cleared': ['미입력 · 전체 지움', '제출 불가: 값 없음'],
                '23-changes-proposed': ['Proposed · 미적용'],
                '24-changes-recorded': ['Recorded · 실행 기록'],
                '25-changes-current': ['Current · 지금 조회'],
                '59-changes-reported': ['Reported · 보고 자료'],
                '36-reconnect': ['전달 결과 불명', '입력 보관 · 제출 잠금']}
    source_map = json.loads((ROOT / 'sources.json').read_text())
    for case in manifest['cases']:
        assert set(case['refs']) <= source_map.keys()
        for v in case['variants']:
            raw = outputs[v['ansi']]
            plain = outputs[v['plain']]
            assert '\x1b[' in raw and SGR.sub('', raw) == plain
            assert '\x1b' not in SGR.sub('', raw), 'non-SGR escape'
            assert all(c == '\n' or ord(c) >= 32 for c in plain), 'unsafe control'
            lines = plain.splitlines()
            assert len(lines) == v['height'] * v['pages']
            assert all(width(s) == v['width'] for s in lines)
            for page_index in range(v['pages']):
                page = lines[page_index*v['height']:(page_index+1)*v['height']]
                joined = ''.join(s.strip() for s in page)
                for phrase in critical.get(case['id'], []):
                    assert phrase.replace(' ', '') in joined.replace(' ', ''), (case['id'], v, phrase)
    return len(manifest['files']), sum(v['pages'] for c in manifest['cases'] for v in c['variants'])

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='verify witnesses and generated file drift')
    parser.add_argument('--show', metavar='CASE', help='print only one ANSI page to stdout')
    parser.add_argument('--width', type=int, default=80, choices=[24,40,80,100])
    parser.add_argument('--height', type=int, help='8 only for 55-tiny at width 24')
    parser.add_argument('--theme', default='dark', choices=PALETTES)
    parser.add_argument('--page', type=int, default=1)
    args = parser.parse_args()
    if args.show:
        case = next((c for c in CASES if c['id'] == args.show), None)
        if case is None: parser.error('unknown case; see README or manifest')
        h = dict(GEOMETRIES)[args.width]
        if args.height is not None:
            if (args.show, args.width, args.height) != ('55-tiny',24,8): parser.error('unsupported custom geometry')
            h = args.height
        fs = frames(case, args.width, h)
        if not 1 <= args.page <= len(fs): parser.error(f'page must be 1..{len(fs)}')
        sys.stdout.write(render(fs[args.page-1],args.width,args.theme)[1])
        return
    outputs, manifest = build()
    count, pages = validate(outputs, manifest)
    if args.check:
        drift = [name for name, value in outputs.items() if not (ROOT/name).exists() or (ROOT/name).read_bytes() != value.encode()]
        actual = {str(p.relative_to(ROOT)) for d in ['ansi','plain'] for p in (ROOT/d).glob('*') if p.is_file()}
        drift += sorted(actual - set(outputs))
        if drift: raise SystemExit('Generated drift: ' + ', '.join(drift))
    else:
        for name, value in outputs.items():
            path = ROOT/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(value.encode())
    print(f"{'PASS' if args.check else 'WROTE'}: {len(CASES)} cases; {count} ANSI/plain files; {pages} ANSI pages; 64 + 10 coverage IDs")

if __name__ == '__main__':
    main()
