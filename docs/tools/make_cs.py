"""Builds docs/cs/index.html, the Czech website, from the English docs/index.html.

Every element with a data-i18n key gets its Czech text from CS below; the head
gets the Czech title, description and links. Run it after changing the
English page:  py -I docs/tools/make_cs.py
"""

import re
from pathlib import Path

DOCS = Path(__file__).resolve().parents[1]
SITE = "https://oondraa.github.io/OZBEAT/"

TITLE = "OZBEAT | Hudební vizualizér pro Windows od oondraa"
DESCRIPTION = (
    "OZBEAT je hudební vizualizér pro Windows zdarma od oondraa. Ukazuje, co hraje "
    "na Spotify, YouTube nebo reproduktorech BluOS: obal alba, texty slovo po slovu, "
    "jak se zpívají, videoklip a vizuály, které se hýbou s hudbou. Je to i spořič obrazovky."
)
SHORT = "OZBEAT od oondraa: obal alba, texty slovo po slovu, videoklipy a vizuály do rytmu. Pro Spotify, YouTube a BluOS."

CS = {
    "heroA": "Vaše hudba", "heroB": "na velké obrazovce.",
    "heroSub": "OZBEAT ukazuje, co hraje váš počítač nebo reproduktory BluOS. S obalem alba, texty slovo po slovu a vizuály, které se hýbou s hudbou.",
    "cta": "Stáhnout pro Windows",
    "share": "Poslat si odkaz do PC",
    "phone": "OZBEAT běží na počítačích s Windows. Pošlete si odkaz a stáhněte ho tam.",
    "ctaScr": "Spořič obrazovky", "ctaScrDl": "Stáhnout OZBEAT.scr",
    "kofi": "Podpořit na Ko-fi",
    "thxT": "Stahování začalo.", "thx": "OZBEAT je zdarma. Jestli se vám líbí, můžete mi koupit kafe.",
    "scrT": "Je to i spořič obrazovky", "scr": "Stejný obraz, když počítač nic nedělá, na všech monitorech. Pohněte myší nebo stiskněte klávesu a zavře se.",
    "st1T": "Stáhněte OZBEAT.scr.", "st1": "Předtím jednou spusťte OZBEAT.exe, ať máte hotové nastavení.",
    "st2T": "Klikněte na něj pravým tlačítkem a zvolte Instalovat.", "st2": "Otevře se nastavení spořiče ve Windows.",
    "st3T": "Nastavte, za jak dlouho se má spustit, a dejte OK.", "st3": "Malý náhled v tom okně zůstane černý. To je v pořádku.",
    "meta": "Windows 10 a 11, 64bit. Nic se neinstaluje. Zdarma pro osobní použití.",
    "f1T": "Cokoliv v počítači", "f1": "Spotify, prohlížeč nebo jakákoliv aplikace v ovládání médií ve Windows.",
    "f2T": "Reproduktory BluOS", "f2": "Přehrávače Bluesound ve vaší síti se najdou samy.",
    "f3T": "Texty i tam, kde jinde nejsou", "f3": "Slovo po slově, přesně když zazní. I u spousty písniček, ke kterým Spotify text nemá.",
    "f4T": "Pozadí z obalu", "f4": "Rozmazaný obal alba plyne za vším ostatním, v barvách písničky.",
    "f5T": "Videoklipy", "f5": "Oficiální klip hraje bez zvuku za texty, v rytmu písničky.",
    "f6T": "Ovládání přímo z obrazovky", "f6": "Pauza, přeskočení a hlasitost na tlačítkách, která se ukážou, když pohnete myší.",
    "duoT": "Dvě obrazovky", "duo": "Stiskněte Ctrl+D. Obal zůstane na monitoru a texty se zvětší na televizi nebo druhé obrazovce.",
    "scr1": "Hlavní obrazovka", "scr2": "Druhá obrazovka",
    "watchT": "Podívejte se, jak to hraje", "watch": "Skutečné záznamy z OZBEATu. Pozadí plyne v barvách obalu a text jde s písničkou.",
    "note": "Když Windows SmartScreen varuje, klikněte na Další informace a pak Přesto spustit. Aplikace zatím není digitálně podepsaná.",
    "aboutT": "Co je OZBEAT?",
    "about1": "OZBEAT je hudební vizualizér pro Windows zdarma, od oondraa (Ondřej Zajíček). Pouštíte si hudbu jako vždycky, na Spotify, YouTube nebo reproduktorech BluOS, a OZBEAT s ní zaplní obrazovku: obal alba jako měkké plynoucí pozadí, texty slovo po slovu, jak se zpívají, videoklip za nimi a vizuály, které se hýbou se zvukem.",
    "about2": "OZBEAT najde texty i u spousty písniček, ke kterým je Spotify nemá, a když přijdou bez časování, načasuje si je sám. Umí dát obal na jednu obrazovku a texty na televizi, běží jako spořič obrazovky ve Windows a rozsvítí světla WLED v barvách obalu. OZBEAT je jeden soubor, nic se neinstaluje.",
    "q1": "Kdo dělá OZBEAT?", "a1": "Ondřej Zajíček, známý jako oondraa. Další jeho práce najdete na",
    "q2": "Funguje OZBEAT se Spotify?", "a2": "Ano. Spotify, YouTube v prohlížeči, jakákoliv aplikace v ovládání médií ve Windows a síťové reproduktory BluOS.",
    "q3": "Je OZBEAT zdarma?", "a3": "Ano, OZBEAT je zdarma pro osobní použití. Stáhněte OZBEAT.exe a spusťte ho.",
    "q4": "Jaké Windows OZBEAT potřebuje?", "a4": "Windows 10 nebo 11, 64bit.",
    "pVidT": "Texty + video", "pVid": "Videoklip za texty", "pLyrT": "Jen texty", "pLyr": "V barvách obalu",
    "releases": "Vydání", "license": "Licence",
}


def replace(page: str, old: str, new: str, count: int = 1) -> str:
    found = page.count(old)
    if found != count:
        raise SystemExit(f"expected {count}x, found {found}x: {old[:80]}")
    return page.replace(old, new)


def main() -> None:
    page = (DOCS / "index.html").read_text(encoding="utf-8")

    # Texts: every data-i18n element holds plain text.
    keys = set()

    def text(m: re.Match) -> str:
        key = m.group(2)
        if key not in CS:
            raise SystemExit(f"no Czech text for {key!r}")
        keys.add(key)
        return m.group(1) + CS[key] + "<"

    page = re.sub(r'(data-i18n="(\w+)"[^>]*>)[^<]*<', text, page)
    unused = set(CS) - keys
    if unused:
        raise SystemExit(f"Czech texts no element uses: {sorted(unused)}")

    page = replace(page, '<html lang="en">', '<html lang="cs">')
    english_title = "OZBEAT | Music visualizer for Windows by oondraa"
    page = replace(page, f"<title>{english_title}</title>", f"<title>{TITLE}</title>")
    page = replace(page, f'content="{english_title}"', f'content="{TITLE}"', 2)
    page = re.sub(r'(<meta name="description" content=")[^"]*', lambda m: m.group(1) + DESCRIPTION, page, count=1)
    page = re.sub(r'(<meta property="og:description" content=")[^"]*', lambda m: m.group(1) + SHORT, page, count=1)
    page = re.sub(r'(<meta name="twitter:description" content=")[^"]*', lambda m: m.group(1) + SHORT, page, count=1)
    page = replace(page, f'<link rel="canonical" href="{SITE}">', f'<link rel="canonical" href="{SITE}cs/">')
    page = replace(page, f'<meta property="og:url" content="{SITE}">', f'<meta property="og:url" content="{SITE}cs/">')
    page = replace(
        page,
        '<meta property="og:locale" content="en_US">\n<meta property="og:locale:alternate" content="cs_CZ">',
        '<meta property="og:locale" content="cs_CZ">\n<meta property="og:locale:alternate" content="en_US">',
    )
    # Already on the Czech page: no redirect to it.
    page = re.sub(r"/\*lang-redirect\*/.*?/\*end\*/", "", page, count=1, flags=re.S)
    page = replace(
        page,
        '<a id="lang-toggle" href="cs/" hreflang="cs" lang="cs" data-lang="cs">Česky</a>',
        '<a id="lang-toggle" href="../" hreflang="en" lang="en" data-lang="en">English</a>',
    )
    page = replace(page, 'data-copied="Link copied"', 'data-copied="Odkaz zkopírován"')
    page = replace(page, f'var url = "{SITE}";', f'var url = "{SITE}cs/";')

    # Files sit one folder up.
    for prefix in ("assets/", "fonts/"):
        page = page.replace(f'"{prefix}', f'"../{prefix}')
    page = page.replace('href="favicon.svg"', 'href="../favicon.svg"')
    page = page.replace('href="sitemap.xml"', 'href="../sitemap.xml"')

    out = DOCS / "cs" / "index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(page, encoding="utf-8", newline="\n")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
