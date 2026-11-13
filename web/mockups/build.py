"""Builds the mockups: inlines lucide icons (and, with --standalone, the CSS and fonts).

python3 web/mockups/build.py <lucide-static icons dir> [--standalone OUT]
"""
import base64, pathlib, re, sys

here = pathlib.Path(__file__).parent
icons = pathlib.Path(sys.argv[1])
src = (here / "network.src.html").read_text()
design = (here.parent.parent / "docs" / "design.md").read_text()
F = re.search(r"const F = '([^']+)'", design).group(1)

def icon(m):
    name, size, cls = m.group(1), m.group(2), m.group(4) or ""
    svg = (icons / f"{name}.svg").read_text()
    body = re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S).group(1)
    body = re.sub(r"\s*\n\s*", "", body)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" class="{cls}" width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" '
            f'stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{body}</svg>')

out = re.sub(r'<i data-i="([\w-]+)" data-s="(\d+)"( data-c="([\w-]+)")?></i>', icon, src).replace("__F__", F)
(here / "network.html").write_text(out)

if "--standalone" in sys.argv:
    css = ""
    for name in ("styles.css", "styles-light.css"):
        c = (here.parent / "src" / name).read_text()
        def font(m):
            data = base64.b64encode((here.parent / "src" / "fonts" / m.group(1)).read_bytes()).decode()
            return f"url(data:font/woff2;base64,{data})"
        css += re.sub(r"url\('\./fonts/([\w-]+\.woff2)'\)", font, c) + "\n"
    out = re.sub(r"<!--CSS-->.*?<!--/CSS-->", lambda _: f"<style>{css}</style>", out, flags=re.S)
    pathlib.Path(sys.argv[sys.argv.index("--standalone") + 1]).write_text(out)
