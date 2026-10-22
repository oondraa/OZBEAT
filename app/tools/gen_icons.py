"""Builds app/src/icons_table.rs and the subset icon font from Phosphor (fill).

A word ending in `*` is a stem: any word starting with it (folded, at least
STEM_MIN letters) and at most STEM_ENDING letters longer gets the icon, so
Czech inflections ("lásk*": láska, lásky, láskou...) need no listing.

Usage: py -I app/tools/gen_icons.py <dir> app/assets/fonts/PhosphorLyrics.ttf app/src/icons_table.rs
where <dir> holds Phosphor-Fill.ttf from @phosphor-icons/web 2.1.1 (src/fill)
saved as Phosphor.ttf, and its style.css saved as phosphor.css with
".ph-fill.ph-" replaced by ".ph.ph-". Needs fonttools.
"""
import re
import sys
import unicodedata
from pathlib import Path

from fontTools import subset

dl = Path(sys.argv[1])
out_font = Path(sys.argv[2])
out_rs = Path(sys.argv[3])

STEM_MIN = 4
STEM_ENDING = 4

# icon name -> words (lowercase, diacritics kept; matching strips them too).
VOCAB = {
    # Love, feelings
    "heart": "love loved lover lovers loving heart hearts kiss kisses lásk* miluj* milova* srdc* srdíčk* zamilova* pusa pusu pusou polib*",
    "heart-break": "heartbreak heartbroken broken zlomen* rozchod* rozešl*",
    "smiley": "smile smiling happy laugh laughing úsměv* šťastn* smích* směju smát joy radost*",
    "smiley-sad": "sad sadness lonely smutn* smutek depres* osaměl*",
    "smiley-angry": "angry mad hate hating hater haters naštvan* nenávid* hejt* vztek*",
    "smiley-nervous": "nervous stress stressed anxiety stres* nervóz* úzkost* fear scared afraid strach* bojím bát",
    "smiley-wink": "wink mrknu mrkám",
    "mask-happy": "mask masks maska masku masky fake faleš*",
    "mask-sad": "lie lies liar lying lež lži lhář* lžeš lhala lhal",
    "hand-heart": "thank thanks grateful díky děkuju děkuji vděčn*",
    "hand-waving": "hello bye goodbye ahoj čau sbohem",
    "hands-clapping": "clap applause tleskat potlesk*",
    "hands-praying": "god pray praying prayer bůh boha bohu bohem bože modlím modlit* modlitb* amen",
    "peace": "peace chill mír míru klid*",
    "infinity": "forever infinity eternity navždy nekonečn* věčn*",
    "sparkle": "shine shining glow magic třpyt* zářím září zářit kouzl* magi*",
    "shooting-star": "wish wishes přání přeju",
    "spiral": "fate destiny osud*",
    "cloud-moon": "dream dreams dreaming dreamer sen snu sny snech sním snít",
    "brain": "mind brain think thinking mysl mysli mozek mozku myslím přemýšl* hlava hlavě hlavu hlavou psycho",
    "flower-lotus": "soul duše duši duší",
    # Body
    "eye": "eye eyes look looking oči očí očima oko vidím vidíš koukám kouk*",
    "ear": "ear ears listen listening ucho uši ušima poslouchej poslouchám slyš*",
    "hand": "hand hands ruka ruku ruce rukou rukama",
    "hand-fist": "fight fighting fist punch fighter boj bojuj* bojovat pěst* rvačk*",
    "tooth": "tooth teeth grillz zuby zubů zubama zubech",
    "heartbeat": "heartbeat pulse tep tepu life living život* žít žijem žijeme",
    "drop-half": "blood bleed bleeding krev krve krví krvi krvác*",
    "drop": "tears tear cry crying cried slzy slz slzami slzách breč* pláč* pláču",
    "barbell": "gym workout fitko posilovn* činky",
    "person-simple-run": "run running runnin utíkám utéct utíkat běžím běžet",
    # People
    "users-three": "friends crew gang squad homies team kamarád* parta partu bráchové bratři family people crowd rodin* lidi lidí lidem dav davy davu",
    "user": "bro brother brácha bráchu brácho bratr bratra bratrem",
    "person": "body bodies tělo těla tělem",
    "gender-female": "girl girls woman women holka holku holky holek holkám žena ženu ženy",
    "gender-male": "guy guys boy boys kluk* chlap* muž mužů muži",
    "baby": "child children kid kids dítě děti dětí mimin*",
    "handshake": "deal deals partner partners dohod* parťák* loyal loyalty loajalit*",
    "crown": "king queen crown boss král krále králem králi králové královn* korun* trůn* šéf šéfa šéfem šéfe",
    # Money, luxury
    "money-wavy": "money cash dollar dollars bucks racks rich peníze peněz* peníz* prachy prachů keš keše bohat* boháč* million millions milion milionu miliony",
    "coins": "gold golden coin coins zlat* mince",
    "diamond": "diamond diamonds ice icy flex flexím flexit diamant* briliant* ring prsten*",
    "hand-coins": "pay paid payment plat*",
    "credit-card": "card cards karta kartu kartou platím",
    "bank": "bank banks banka banku bance",
    "piggy-bank": "savings spořit úspor*",
    "vault": "vault trezor*",
    "receipt": "bill bills účet účty účtenk*",
    "currency-btc": "bitcoin btc crypto krypto*",
    "currency-eur": "euro eura eur",
    "shopping-cart": "shopping buy buying nakupuju koupím koupit nákup*",
    "storefront": "shop store obchod* krám",
    "briefcase": "work job business hustle hustling grind práce práci prací makám makat byznys*",
    "watch": "watch rolex hodinky hodinkách",
    "handbag": "bag bags gucci louis prada balenciaga taška tašku kabelk*",
    "t-shirt": "shirt hoodie drip outfit triko trička mikin* oblek*",
    "dress": "dress šaty šatech",
    "pants": "pants jeans kalhot* džíny rifle",
    "baseball-cap": "cap kšilt* čepic*",
    "eyeglasses": "glasses brýle brejle",
    "sneaker": "shoes sneakers boty botách tenisk* jordan* nike nikes",
    "link": "chain chains řetěz* řetízek",
    # Vehicles, travel
    "car-profile": "car cars drive driving driver auto auta autem autě autu kára káru fáro fára fárem bourák* benz mercedes merc bmw audi porsche lambo lamborghini ferrari tesla lincoln",
    "gauge": "speed fast rychl* tlak*",
    "steering-wheel": "steering wheel volant*",
    "gas-pump": "gas benzín* tankuju",
    "motorcycle": "motorcycle motork* motorce",
    "bicycle": "bicycle kolo",
    "taxi": "taxi uber",
    "airplane-tilt": "plane planes fly flying flight jet letadl* letí letím letíme letět létat airport letišt*",
    "train": "train vlak* metro metrem",
    "sailboat": "boat yacht loď lodi lodí jachta jachtu",
    "anchor": "anchor kotva",
    "suitcase": "travel trip suitcase cestování cestuju kufr* dovolen*",
    "paper-plane-tilt": "send sent pošli poslat poslal",
    "compass": "compass kompas směr*",
    "map-trifold": "map maps mapa mapě mapu",
    "signpost": "direction rozcestí",
    "rocket-launch": "rocket rockets raket*",
    # Places
    "house": "home house domov* domů doma dům domě barák* byt bytě střech*",
    "buildings": "city town cities downtown block město města městem městě měst praha prahu prahy brno brna blok bloku bloky sídlišt* panelák*",
    "road-horizon": "road roads street streets highway hood cesta cestu cesty cestě dálnic* ulic*",
    "bridge": "bridge bridges mostu mostem mosty",
    "castle-turret": "castle hrad*",
    "church": "church churches kostel*",
    "cross": "cross jesus kříž* ježíš*",
    "door": "door doors dveře dveří dveřmi",
    "stairs": "stairs schody schodech",
    "couch": "couch sofa gauč*",
    "bed": "bed sleep sleeping postel* spát spím usínám hotel hotelu",
    "shower": "shower sprch*",
    "island": "island beach ostrov* pláž*",
    "globe-hemisphere-west": "world worldwide svět světa světem světě světu earth ground zem zemi země zemí",
    "planet": "planet planets planet* vesmír* space galaxy galaxie",
    "alien": "alien aliens mimozem* ufo",
    # Nature, weather
    "moon-stars": "night nights tonight midnight noc noci nocí nocích půlnoc* nočn*",
    "moon": "moon měsíc měsíce měsícem měsíci luna lunu dark darkness tma tmě tmou tmy",
    "sun": "sun sunshine sunny summer slunc* sluníčk* léto léta létě",
    "sun-horizon": "morning sunrise sunset ráno svítání",
    "star": "star stars superstar hvězd*",
    "cloud": "cloud clouds sky mrak* oblak* nebe nebi nebem obloh*",
    "cloud-rain": "rain raining rainy déšť deště deštěm deštiv* prší pršelo",
    "umbrella": "umbrella deštník*",
    "snowflake": "snow cold frozen sníh sněh* zima zimě zimu zimou mráz* mrazu led ledu ledem",
    "lightning": "lightning thunder storm energy blesk* bouřk* bouře energi*",
    "tornado": "tornado hurricane uragán*",
    "wind": "wind vítr větru větrem air vzduch*",
    "rainbow": "rainbow duha duhu duhy",
    "fire": "fire flame flames burn burning burned lit hot oheň ohně ohněm ohni hoří hořím hořet hořel pálí pálim pálím pálit plamen*",
    "waves": "ocean sea waves wave moře mořem oceán* vlny vlna vlnu vlnách water voda vodu vody vodou",
    "mountains": "mountain mountains hora hory hor horách vrchol*",
    "tree": "tree trees forest strom* les lesy lese",
    "plant": "plant plants rostlin* květináč*",
    "cactus": "cactus kaktus*",
    "flower-tulip": "flower flowers rose roses květin* kytk* růže růži růží",
    "leaf": "weed blunt joint ganja tráva trávu trávou jointa jointy",
    "feather": "feather peří pírko",
    # Animals
    "dog": "dog dogs puppy pes psa psem psi pejsk*",
    "cat": "cat cats kitty koček kočk* kočičk*",
    "bird": "bird birds pták* ptáci ptáčk* free freedom svobod*",
    "butterfly": "butterfly butterflies motýl*",
    "fish": "fish ryba ryby rybu",
    "horse": "horse horses kůň koně koni",
    "bug": "bug bugs brouk* hmyz*",
    "paw-print": "animal animals zvíře zvířata",
    # Food, drinks, drugs
    "coffee": "coffee káva kafe kafíčko espresso",
    "pizza": "pizza pizzu",
    "hamburger": "burger burgers food eat eating hungry hamburger* jídlo jídla jím žrát hlad*",
    "bread": "bread chleba chleb*",
    "ice-cream": "zmrzlin*",
    "cake": "cake birthday dort* narozenin*",
    "fork-knife": "dinner restaurant večeř* restaurac*",
    "cooking-pot": "cook cooking vařím vařit kuchyň*",
    "wine": "wine víno vína vínem",
    "champagne": "champagne toast šampaňsk* šampus*",
    "beer-bottle": "beer beers pivo piva pivem pivko",
    "martini": "drink drinks drinking cocktail drunk pití piju chlast* opil* ožral* alkohol* panák* baru barem",
    "cigarette": "cigarette cigarettes smoke smoking cigaret* cigár* cigo cíga kouř* hulím hulit hulíme",
    "pill": "pill pills drugs drug prášk* drog* perník* cocaine coke kokain* koks*",
    "syringe": "needle jehla jehlu",
    # Danger, law
    "skull": "dead death die dying kill killer killing smrt smrti mrtv* umřu umřít umírám umřel zabij* zabít vrah",
    "ghost": "ghost ghosts duch duchové duchy přízrak*",
    "knife": "knife nůž nože nožem kudl* betray zrad*",
    "sword": "sword war meč meče mečem válk* války",
    "crosshair": "target aim gun guns shoot shooting shot zbraň* pistol* glock* střel*",
    "bomb": "bomb bombs bomba bombu výbuch* bomb*",
    "shield": "protect security ochran* bezpeč* štít*",
    "police-car": "police cops cop popo policie policajt* fízl* benga",
    "siren": "siren alarm sirén* poplach*",
    "lock": "lock locked jail prison zámek zamčen* vězení kriminál*",
    "key": "key keys klíč*",
    "gavel": "judge court law soud* zákon* soudce",
    "warning": "danger warning nebezpe* pozor",
    "first-aid": "doctor hospital nemocnic* doktor* sanitk*",
    "bandaids": "hurt pain wound wounds bolest* bolí zraněn*",
    "virus": "virus covid sick ill nemoc*",
    "thermometer": "fever horečk*",
    # Time
    "clock": "time hours minute minutes čas času časem hodiny hodin minut* sekund*",
    "clock-counter-clockwise": "memory memories vzpomín* vzpomínk*",
    "hourglass": "wait waiting čekám čekat čeká čekej",
    "alarm": "budík* wake",
    "calendar-blank": "day days tomorrow yesterday week weekend den dny zítra včera týden víkend* year rok roku roky zejtra",
    # Music, media
    "music-notes": "music song songs track tracks hudb* písničk* píseň písně beat beaty beatu melodi* rytmus",
    "microphone-stage": "mic microphone rap rapper rappers bars mikrofon* mikrák* rapuju rapuje rapovat raper* verše verš hlas hlasem hlasu studio studiu studia zpěvák* singer",
    "headphones": "headphones sluchátk*",
    "speaker-hifi": "speaker speakers bass reprák* repro basy",
    "speaker-x": "silence quiet ticho tichu potichu",
    "megaphone": "loud shout scream křič* křik řvu řvát",
    "radio": "radio rádio rádia rádiu",
    "vinyl-record": "vinyl record records album alba deska desku desky",
    "guitar": "guitar kytar*",
    "piano-keys": "piano klavír*",
    "disco-ball": "dance dancing dancer club clubs klub* tanc* tančím tančit diskotéka",
    "confetti": "party parties celebrate párty party* oslav* mejdan*",
    "ticket": "ticket tickets lístek lístky vstupenk*",
    "camera": "camera photo photos picture foto fotk* kamer* fotografi* selfie",
    "video-camera": "video videa videoklip* klip klipy klipu natáčím",
    "film-slate": "movie movies film filmu filmy filmem",
    "popcorn": "popcorn",
    "television": "tv television televiz* telka",
    "game-controller": "game games gaming gamer playstation hra hry hru",
    "dice-five": "dice luck lucky gamble casino kostk* štěstí kasino",
    "poker-chip": "poker chips",
    "soccer-ball": "football soccer fotbal*",
    "basketball": "basketball basket nba",
    "trophy": "trophy win winning winner champion champions vítěz* vyhr* šampion*",
    "medal": "medal medaile",
    "puzzle-piece": "puzzle missing chybíš chybí chybět",
    "paint-brush": "art artist umění umělec malovat",
    # Talk, tech
    "phone": "phone phones call calling calls iphone telefon* mobil* volám voláš volá zavolej",
    "bell": "bell bells zvon*",
    "envelope-simple": "letter message messages sms zpráv* dopis*",
    "chat-circle-dots": "text texting chat talk talking píšu píšeš psát mluv* kecy kecám word words slovo slova slov slovy",
    "seal-check": "truth true real pravd* fakta",
    "laptop": "laptop computer notebook počítač* pc",
    "wifi-high": "online wifi internet internetu",
    "instagram-logo": "insta instagram ig",
    "youtube-logo": "youtube",
    "spotify-logo": "spotify",
    "tiktok-logo": "tiktok",
    "snapchat-logo": "snapchat snap",
    "thumbs-up": "lajk* lajky",
    "robot": "robot robots robot*",
    "battery-charging": "battery charge baterk* nabít",
    "plug": "plug zásuvk*",
    "gear": "machine engine stroj* motor*",
    "wrench": "fix opravit opravím",
    "hammer": "hammer build kladiv* stavím budovat",
    "ladder": "ladder žebřík*",
    "lightbulb": "idea ideas light nápad* světl*",
    "palette": "color colors colour barv*",
    "graduation-cap": "school college škol* univerzit*",
    "book-open": "book books kniha knihu knihy knížk*",
    "pencil-simple": "write writing pen pero tužk* napíšu",
    "target": "goal goals cíl cíle cílem",
    "flag": "flag flags vlajk*",
    "trend-up": "rise rising growth růst roste stoupám",
    "trend-down": "fall falling padám padat",
    "gift": "gift gifts present dárek dárky dárku dárků dárkem",
    "fingerprint": "identity identit*",
    "identification-badge": "name jméno jména jménem",
    "yin-yang": "balance rovnováh*",
}

css = (dl / "phosphor.css").read_text(encoding="utf-8")
codes = dict(re.findall(r'\.ph\.ph-([a-z0-9-]+):before\s*\{\s*content:\s*"\\([0-9a-f]+)"', css))
missing = [n for n in VOCAB if n not in codes]
if missing:
    sys.exit(f"unknown icons: {missing}")


def fold(word):
    nfd = unicodedata.normalize("NFD", word.lower())
    return "".join(c for c in nfd if unicodedata.category(c) != "Mn")


exact = {}
stems = {}
problems = []
for name, words in VOCAB.items():
    for w in words.split():
        if not re.fullmatch(r"[a-zà-ž']+\*?", w):
            problems.append(f"bad word {w!r} for {name}")
            continue
        if w.endswith("*"):
            stem = fold(w[:-1])
            if len(stem) < STEM_MIN:
                problems.append(f"stem {w} for {name} is too short")
            if stems.get(stem, name) != name:
                problems.append(f"stem {w} listed for {stems[stem]} and {name}")
            stems[stem] = name
        else:
            f = fold(w)
            if exact.get(f, name) != name:
                problems.append(f"{w} listed for {exact[f]} and {name}")
            exact[f] = name


def by_stem(word):
    for k in range(len(word), STEM_MIN - 1, -1):
        if len(word) - k <= STEM_ENDING and word[:k] in stems:
            return stems[word[:k]]
    return None


# An exact word another icon's stem would also take is fine (exact wins),
# but worth a look.
for f, name in exact.items():
    other = by_stem(f)
    if other and other != name:
        print(f"note: {f} -> {name}, stem says {other}")
if problems:
    sys.exit("\n".join(problems))

used = sorted({int(codes[n], 16) for n in VOCAB})
opts = subset.Options()
opts.layout_features = []
opts.name_IDs = ["*"]
opts.notdef_outline = True
font = subset.load_font(str(dl / "Phosphor.ttf"), opts)
sub = subset.Subsetter(opts)
sub.populate(unicodes=used)
sub.subset(font)
subset.save_font(font, str(out_font), opts)

rows = []
for name, words in VOCAB.items():
    quoted = ", ".join(f'"{w}"' for w in words.split())
    rows.append(f"    // {name}\n    ('\\u{{{codes[name]}}}', &[{quoted}]),")
out_rs.write_text(
    "// Generated from Phosphor's icon names; words ending in `*` are stems.\n"
    "pub(super) const ICONS: &[(char, &[&str])] = &[\n" + "\n".join(rows) + "\n];\n",
    encoding="utf-8",
)
print(len(VOCAB), "icons,", len(exact), "words,", len(stems), "stems,", out_font.stat().st_size, "bytes")
