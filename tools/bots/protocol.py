"""Turns the protocol of a bot run (PAGAN_BOT_LOG, one JSON event per line)
into one HTML page to look at the game it played: the way it went on each
map (seen as in the game, from the south-east, coloured from early to
late), marks where something happened (deaths, rule breaches, things that
hurt, fights, talks, dead ends), the hit points over time and every event
in order, to filter by kind.
usage: python protocol.py <run.jsonl> <out.html>"""
import html
import json
import sys
from collections import defaultdict

src, dst = sys.argv[1], sys.argv[2]
events = []
with open(src, encoding="utf-8") as f:
    for line in f:
        line = line.strip()
        if line:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass  # (a run cut short: the last line half written)

# what the marks on the map look like: colour, and the kinds they stand for
MARKS = {
    "death": "#d62728",
    "bug": "#e377c2",
    "harmful": "#ff7f0e",
    "fight": "#8c1c13",
    "flee": "#8c1c13",
    "fight-stuck": "#8c1c13",
    "answer": "#2ca02c",
    "done": "#1f77b4",
    "not-reached": "#bcbd22",
    "walk-stuck": "#bcbd22",
    "map": "#17becf",
}
# what the timeline leaves out (there are very many of them)
QUIET = {"pos", "walk-failed"}
BASE = {"t", "tick", "k", "map", "at", "hp"}


def iso(x, y):
    """World position to the picture: as the game shows it."""
    return (x - y) / 4, (x + y) / 8


def details(e):
    parts = []
    for k, v in e.items():
        if k in BASE:
            continue
        if isinstance(v, list):
            v = "; ".join(str(i) for i in v) if v else "-"
        parts.append(f"<b>{html.escape(k)}</b> {html.escape(str(v))}")
    return " · ".join(parts)


def map_svg(mapnum, evs, t_max):
    pts = [(e["t"], *iso(e["at"][0], e["at"][1])) for e in evs]
    xs = [p[1] for p in pts]
    ys = [p[2] for p in pts]
    x0, x1, y0, y1 = min(xs) - 40, max(xs) + 40, min(ys) - 40, max(ys) + 40
    w, h = x1 - x0, y1 - y0
    out = [f'<svg viewBox="{x0:.0f} {y0:.0f} {w:.0f} {h:.0f}" class="map" role="img" aria-label="Weg auf Karte {mapnum}">']
    # the way, in pieces coloured by time (a jump - a teleport, a reload - is not drawn)
    prev = None
    for e in evs:
        if e["k"] not in ("pos", "map", "start", "reload", "goal", "done"):
            continue
        p = iso(e["at"][0], e["at"][1])
        if prev and e["k"] not in ("map", "reload") and abs(p[0] - prev[0]) + abs(p[1] - prev[1]) < 300:
            hue = 220 - 220 * e["t"] / max(t_max, 1)
            out.append(f'<line x1="{prev[0]:.1f}" y1="{prev[1]:.1f}" x2="{p[0]:.1f}" y2="{p[1]:.1f}" stroke="hsl({hue:.0f} 70% 50%)" />')
        prev = p
    size = max(w, h) / 120
    for e in evs:
        colour = MARKS.get(e["k"])
        if not colour:
            continue
        if e["k"] == "done" and not e.get("changes"):
            continue
        x, y = iso(e["at"][0], e["at"][1])
        r = size * (1.8 if e["k"] in ("death", "bug", "harmful", "fight-stuck", "walk-stuck", "not-reached") else 1)
        tip = html.escape(f'{e["k"]} · t {e["t"]} · hp {e["hp"]} · ' + "; ".join(f"{k} {v}" for k, v in e.items() if k not in BASE))
        out.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.1f}" fill="{colour}" stroke="var(--bg)" stroke-width="{size / 3:.1f}" class="m-{e["k"]}"><title>{tip}</title></circle>')
    out.append("</svg>")
    return "\n".join(out)


def hp_svg(states):
    if len(states) < 2:
        return ""
    t_max = states[-1]["t"] or 1
    top = max(max(s.get("max_hp", s["hp"]) for s in states), 1)
    pts = " ".join(f'{600 * s["t"] / t_max:.1f},{80 - 76 * s["hp"] / top:.1f}' for s in states)
    return f'<svg viewBox="0 0 600 84" class="hp" role="img" aria-label="Trefferpunkte über die Zeit"><polyline points="{pts}" fill="none" stroke="#d62728" stroke-width="1.5"/><text x="2" y="10">{top} HP</text></svg>'


t_max = max((e["t"] for e in events), default=1)
by_map = defaultdict(list)
for e in events:
    by_map[e["map"]].append(e)
start = next((e for e in events if e["k"] == "start"), {})
summary = next((e for e in events if e["k"] == "summary"), {})
states = [e for e in events if e["k"] in ("state", "start")]
kinds = sorted({e["k"] for e in events} - QUIET)

rows = []
for e in events:
    if e["k"] in QUIET:
        continue
    rows.append(f'<tr data-k="{html.escape(e["k"])}"><td>{e["t"]}</td><td><span class="k" style="--c:{MARKS.get(e["k"], "#888")}">{html.escape(e["k"])}</span></td>'
                f'<td>{e["map"]}</td><td>{e["at"][0]},{e["at"][1]},{e["at"][2]}</td><td>{e["hp"]}</td><td>{details(e)}</td></tr>')

maps_html = "\n".join(f'<section><h2>Karte {m} <small>({len(evs)} Ereignisse)</small></h2>{map_svg(m, evs, t_max)}</section>' for m, evs in sorted(by_map.items()))
legend = " ".join(f'<span class="k" style="--c:{c}">{k}</span>' for k, c in MARKS.items())
filters = " ".join(f'<label><input type="checkbox" checked value="{html.escape(k)}"> {html.escape(k)}</label>' for k in kinds)

page = f"""<!doctype html>
<html lang="de"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Bot-Protokoll</title>
<style>
:root {{ --bg:#fff; --fg:#1d1d1f; --muted:#6e6e73; --line:#e5e5ea; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#1c1c1e; --fg:#f2f2f7; --muted:#a1a1a6; --line:#3a3a3c; }} }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, sans-serif; margin:0 auto; padding:16px; max-width:1200px; }}
h1 {{ font-size:20px; margin:0 0 4px; }} h2 {{ font-size:16px; margin:24px 0 8px; }} small, .muted {{ color:var(--muted); font-weight:normal; }}
svg.map {{ width:100%; max-height:80vh; border:1px solid var(--line); border-radius:8px; }}
svg.map line {{ stroke-width:3; stroke-linecap:round; }}
svg.hp {{ width:100%; height:90px; border:1px solid var(--line); border-radius:8px; }} svg.hp text {{ fill:var(--muted); font-size:9px; }}
.k {{ display:inline-block; padding:0 6px; border-radius:4px; border-left:4px solid var(--c); background:color-mix(in srgb, var(--c) 12%, transparent); white-space:nowrap; }}
table {{ border-collapse:collapse; width:100%; font-size:13px; }} td, th {{ border-bottom:1px solid var(--line); padding:4px 6px; text-align:left; vertical-align:top; }}
td:last-child {{ word-break:break-word; }} .wrap {{ overflow-x:auto; }}
#filters label {{ margin-right:10px; white-space:nowrap; }}
</style></head><body>
<h1>Bot-Protokoll <small>Seed {start.get("seed", "?")}, {start.get("ticks", "?")} Ticks</small></h1>
<p>{html.escape(summary.get("text", "(kein Abschluss: Lauf abgebrochen)"))}<br><span class="muted">{html.escape(summary.get("coverage", ""))}</span></p>
<h2>Trefferpunkte</h2>{hp_svg(states)}
<p class="muted">Weg: blau = früh, rot = spät. Markierungen (Maus darüber für Einzelheiten): {legend}</p>
{maps_html}
<h2>Ereignisse</h2>
<p id="filters">{filters}</p>
<div class="wrap"><table><thead><tr><th>Tick</th><th>Art</th><th>Karte</th><th>Ort</th><th>HP</th><th>Was</th></tr></thead><tbody>
{"".join(rows)}
</tbody></table></div>
<script>
document.getElementById("filters").addEventListener("change", () => {{
  const on = new Set([...document.querySelectorAll("#filters input:checked")].map(i => i.value));
  document.querySelectorAll("tbody tr").forEach(r => r.hidden = !on.has(r.dataset.k));
}});
</script>
</body></html>
"""
with open(dst, "w", encoding="utf-8") as f:
    f.write(page)
print(f"{len(events)} events, {len(by_map)} maps -> {dst}")
