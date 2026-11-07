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
    "jak se zpívají, styly textů jako z editů, videoklip a vizuály, které se hýbou s hudbou. Je to i spořič obrazovky."
)
SHORT = "OZBEAT od oondraa: živé texty slovo po slovu, styly textů a videoklipy. Pro Spotify, YouTube a BluOS."

CS = {
    "navHow": "Jak to funguje", "navFeat": "Funkce", "navScr": "Spořič", "ctaShort": "Stáhnout",
    "heroA": "Vaše hudba", "heroB": "na velké obrazovce.",
    "heroSub": "Texty slovo po slovu, přesně když zazní. Přes videoklip, na monitoru nebo na televizi. Ke všemu, co hraje váš počítač nebo reproduktory BluOS.",
    "cta": "Stáhnout pro Windows",
    "share": "Poslat si odkaz do PC",
    "phone": "OZBEAT běží na počítačích s Windows. Pošlete si odkaz a stáhněte ho tam.",
    "kofi": "Podpořit na Ko-fi",
    "meta": "Zdarma · Windows 10 a 11 · Jeden soubor, nic se neinstaluje",
    "drag": "Táhněte a rozhlédněte se · každý klip je skutečný záznam",
    "flowE": "Jak to funguje", "flowT": "Hraje s tím, co už používáte",
    "flow": "Žádné účty, žádné přihlašování. OZBEAT poslouchá ovládání médií ve Windows a reproduktory BluOS ve vaší síti, najde texty i klip a dá je, kam chcete.",
    "nBrowser": "YouTube v prohlížeči", "nApps": "Jakákoliv aplikace", "nBluos": "Reproduktory Bluesound",
    "nSmtc": "Ovládání médií ve Windows", "nCore": "texty · klip · rytmus",
    "oScreen": "Celá obrazovka", "oTv": "Televize nebo druhý monitor",
    "oSaver": "Spořič obrazovky", "oWled": "Světla WLED",
    "flowNote": "Texty jsou z LRCLIB a časují se slovo po slovu; videoklipy z vlastních YouTube kanálů interpretů.",
    "featE": "Funkce", "featT": "Vyberte, jak písnička vypadá",
    "feat": "Stiskněte Tab a vyberte styl z mřížky náhledů. Každý z nich jde s písničkou slovo po slovu.",
    "b1T": "Živé texty", "b1": "Verš se skládá jako plakát. Každé slovo se hýbe podle toho, co znamená a jak se zpívá.",
    "bNeonT": "Neon za deštěm", "bNeon": "Zářící slova za mokrým oknem.",
    "b3T": "CRT", "b3": "Těžké písmo na zářící obrazovce, vypouklé pod sklem.",
    "b4T": "Psaní a CD", "b4": "Jako lyric edity na vašem feedu.",
    "b6T": "Přes videoklip", "b6": "Oficiální klip, bez zvuku a v rytmu, pod slovy.",
    "b7T": "Dvě obrazovky", "b7": "Obal na monitoru, texty velké na televizi.",
    "b8T": "Kapesní přehrávač", "b8": "Verš na podsvíceném displeji, i s kolečkem.",
    "b9T": "Ovládání přímo z obrazovky", "b9": "Pauza, přeskočení a hlasitost se ukážou, když pohnete myší. Texty se zpožďují? Alt+šipky je posunou, pro každou písničku zvlášť.",
    "scrE": "Spořič", "scrT": "Je to i spořič obrazovky",
    "scr": "Stejný obraz, když počítač nic nedělá, na všech monitorech. Pohněte myší nebo stiskněte klávesu a zavře se.",
    "ctaScrDl": "Stáhnout OZBEAT.scr",
    "st1T": "Stáhněte OZBEAT.scr.", "st1": "Předtím jednou spusťte OZBEAT.exe, ať máte hotové nastavení.",
    "st2T": "Klikněte na něj pravým tlačítkem a zvolte Instalovat.", "st2": "Otevře se nastavení spořiče ve Windows.",
    "st3T": "Nastavte, za jak dlouho se má spustit, a dejte OK.", "st3": "Malý náhled v tom okně zůstane černý. To je v pořádku.",
    "aboutE": "O aplikaci", "aboutT": "Co je OZBEAT?",
    "about1": "OZBEAT je hudební vizualizér pro Windows zdarma, od oondraa (Ondřej Zajíček). Pouštíte si hudbu jako vždycky, na Spotify, YouTube nebo reproduktorech BluOS, a OZBEAT s ní zaplní obrazovku: texty slovo po slovu, jak se zpívají, ve stylu, který si vyberete, přes videoklip nebo v barvách obalu.",
    "about2": "Najde texty i u spousty písniček, ke kterým je Spotify nemá, a když přijdou bez časování, načasuje si je sám. Umí dát obal na jednu obrazovku a texty na televizi, běžet jako spořič obrazovky a rozsvítit světla WLED v barvách obalu. OZBEAT je jeden soubor, nic se neinstaluje.",
    "q1": "Kdo dělá OZBEAT?", "a1": "Ondřej Zajíček, známý jako oondraa. Další jeho práce najdete na",
    "q2": "Funguje OZBEAT se Spotify?", "a2": "Ano. Spotify, YouTube v prohlížeči, jakákoliv aplikace v ovládání médií ve Windows a síťové reproduktory BluOS. Bez účtu a přihlašování.",
    "q3": "Je OZBEAT zdarma?", "a3": "Ano, OZBEAT je zdarma pro osobní použití. Stáhněte OZBEAT.exe a spusťte ho.",
    "q4": "Jaké Windows OZBEAT potřebuje?", "a4": "Windows 10 nebo 11, 64bit.",
    "note": "Když Windows SmartScreen varuje, klikněte na Další informace a pak Přesto spustit. Aplikace zatím není digitálně podepsaná.",
    "finalT": "Pusťte to nahlas.", "final": "Zdarma pro Windows. Jeden soubor, nic se neinstaluje.",
    "thxT": "Stahování začalo.", "thx": "OZBEAT je zdarma. Jestli se vám líbí, můžete mi koupit kafe.",
    "releases": "Vydání", "license": "Licence", "privacy": "Soukromí",
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
    page = replace(page, 'href="privacy/"', 'href="../privacy/"')

    out = DOCS / "cs" / "index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(page, encoding="utf-8", newline="\n")
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
