//! Lyrics that live, the way lyric edits do it: the line being sung is set
//! like a poster, the words that matter huge and the small ones small, each
//! word coming in its own way as it is sung. What a word says picks how it
//! moves ("fire" flickers, "down" drops in, "money" spins in gold), how it is
//! sung picks how big it is (held notes grow, shouted words slam), and the
//! beat shakes and punches the whole thing.

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use eframe::egui::{
    Color32, FontFamily, FontId, Galley, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2,
    emath::Rot2,
    epaint::Vertex,
    text::{LayoutJob, TextFormat},
};

use super::{BOLD, HEAVY, REGULAR, lerp_color, word_colors, wobble};
use crate::anim::{ease_in_cubic, ease_out_cubic, remap, spring};
use crate::icons;

pub(super) mod looks;
use looks::Look;

/// What the music is doing right now, each 0..1.
#[derive(Clone, Copy, Default)]
pub(super) struct Groove {
    /// Jumps on a beat, then eases back down.
    pub beat: f32,
    pub bass: f32,
}

/// Size of an ordinary word, against the lines' size at the same spot.
pub(super) const LIVE_SCALE: f32 = 1.5;
/// How tall the poster may grow, in ordinary word sizes.
const LIVE_HEIGHT: f32 = 4.4;
/// Room beside a word for its symbol, in the word's heights.
const SYMBOL_ROOM: f32 = 1.05;
/// How long the last line takes to blow away once the next one starts.
const LIVE_EXIT: f32 = 0.35;

/// How a word moves, by what it says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mood {
    Fire,
    Cold,
    Fall,
    Rise,
    Shake,
    Money,
    Love,
    Dark,
    Wave,
    Fast,
    Stop,
    /// Big things: grows on past its place.
    Grow,
    Spin,
    /// Light: flashes white.
    Flash,
    /// Party: letters hop to the beat.
    Bounce,
    /// Slow motion: drifts in stretched.
    Slow,
    /// Leaving by plane: one flies in and takes the word away.
    FlyAway,
    /// Leaving by car: one drives up and off with the word.
    DriveAway,
    /// Leaving on foot: the word walks off.
    Away,
    /// Sex and such: letters sway, in pink and violet.
    Freaky,
    /// Not real: letters glitch and drop out.
    Fake,
    /// Sorry: the word bows its head.
    Sorry,
    /// Split: the word comes apart in the middle.
    Split,
    /// Searching: letters turn up one by one.
    Seek,
    /// Back: comes in from the right, rewinding.
    Back,
}

/// Words by mood, the first listing winning. Words are matched folded
/// (lowercase, no diacritics); English endings (-s, -ing, -ed) come off by
/// themselves. Folding makes some words one ("moře", "more"), so an entry can say more:
/// - `stem*`: word forms too, so Czech ones need no listing;
/// - `=word`: only as written, diacritics and all ("=moře" isn't "more");
/// - `cs:word`: only in a Czech or Slovak song ("more" there is "bro");
/// - `en:word`: only in any other.
const MOODS: &[(Mood, &[&str])] = &[
    (Mood::FlyAway, &[
        "odlet*", "odlét*", "ulet*", "ulét*", "ulít*", "odfrč*", "takeoff", "liftoff",
    ]),
    (Mood::DriveAway, &[
        "odjed*", "odjíž*", "odjel*", "ujed*", "ujel*", "ujíž*", "skrrt", "skrr", "vroom", "drove",
    ]),
    (Mood::Away, &[
        "odejd*", "odejít", "odcház*", "odešel", "odešla", "odešli", "odchod*", "pryč", "away", "leave", "leaving",
        "gone", "bye", "goodbye", "sbohem", "zmiz*", "vanish*", "disappear*",
        "vypadn*", "vypadni",
    ]),
    (Mood::Fire, &[
        "fire", "fiery", "burn*", "flame*", "hot", "hotter", "heat", "blaze*", "lit", "inferno", "smoke*", "smokin",
        "spark*", "ignite*", "explode*", "explosion", "bomb*", "boom", "gasoline", "lighter", "ash", "ashes",
        "sun", "sunshine", "summer", "oheň", "ohně", "ohněm", "ohni", "ohniv*", "hoří", "hořím", "hoříš", "hoříme",
        "hořet", "shořel*", "plamen*", "pálí", "pálím", "pálit", "spálí*", "žár", "žhav*", "kouř*", "jiskr*",
        "výbuch*", "slunce", "slunko", "léto", "horko", "horký", "horká",
        "párno", "vedro", "vedru",
    ]),
    (Mood::Cold, &[
        "ice", "icy", "iced", "cold", "colder", "freeze", "freezing", "frozen", "frost*", "snow*", "winter", "numb",
        "cs:led", "cs:ledu", "ledov*", "mráz", "mraz*", "zmrzl*", "zima", "zimu", "zimy", "chlad*", "studená", "studený", "studené", "studenou", "sníh",
        "sněh*", "vločk*",
    ]),
    (Mood::Fall, &[
        "down", "fall*", "fell", "drop*", "low", "lower", "sink*", "crash*", "under", "underground", "deep", "deeper",
        "ground", "floor", "gravity", "bottom", "dolů", "dole", "cs:pád", "cs:pádu", "padám", "padá", "padáš", "padat",
        "padl*", "spadl*", "spadn*", "klesá*", "dno", "cs:dna", "dnu", "zem", "zemi", "zemí", "podlah*", "hloub*",
        "broke", "chudý", "chudej", "chudá", "chudák*",
        "daně", "cs:daň", "daní", "taxes", "tax",
    ]),
    (Mood::Rise, &[
        "up", "high", "higher", "highest", "sky", "skies", "fly", "flying", "flew", "wings", "rise", "rising",
        "heaven", "moon", "star", "stars", "top", "above", "climb*", "cloud*", "space", "orbit", "nahoru", "nahoře",
        "výš", "vysoko", "nebe", "nebi", "nebes*", "letím", "letíme", "letíš", "létám", "létat", "vzlét*",
        "křídl*", "výšk*", "hvězd*", "měsíc", "mrak*", "vesmír*", "stoupá*",
        "hore", "vrchol*", "rooftop", "free", "freedom", "svobod*", "duše", "duši", "soul", "ráno", "morning", "sunrise",
        "zítřek", "zítřka", "zítra", "zejtra", "tomorrow", "ráno", "ránem",
        "=líp", "lepší", "better", "hero", "heroes", "hrdin*",
    ]),
    (Mood::Shake, &[
        "scream*", "shout*", "yell*", "crazy", "insane", "psycho", "shake*", "rage", "wild", "kill*", "savage",
        "beast", "monster", "riot", "war", "fight*", "punch", "thunder", "earthquake", "panic", "chaos", "mad",
        "angry", "hate*", "fuck*", "shit", "damn", "bitch*", "křič*", "řvu", "řveš", "řve", "šílen*", "blázn*",
        "blázen", "vztek*", "zuří*", "divok*", "třes*", "válk*", "rvačk*", "panik*", "hrom", "nasran*", "kurva",
        "kurvy", "píča", "píčo", "kokot*", "zmrd*", "sráč*", "hovn*", "sakra", "doprdele",
        "strach*", "problem*", "problém*", "glitch*", "damage", "trouble", "beef", "džungl*", "jungle", "stres*",
        "nenávid*", "nesnáš*", "nesnes*", "bojím", "bojíš", "bojí", "cs:bát", "scared", "afraid", "fear", "sračk*",
        "ekzém", "ekzému", "vyjeban*", "vyjeb*", "cs:kár", "hajzl*", "závisl*", "addict*", "seru", "sereš", "serem", "serou", "nasrat", "jebat", "jebu", "jebe", "jebem", "jebej", "jebal*", "ojeb*", "jebnut*",
    ]),
    (Mood::Money, &[
        "money", "cash", "gold", "golden", "rich", "bands", "racks", "diamond*", "dollar*", "bling", "drip*",
        "flex*", "chain*", "rolex", "bag", "bags", "bread", "bank*", "euro", "euros", "million*", "billion*",
        "mula", "guap", "stack*", "paid", "coin*", "peníz*", "peněz", "prachy", "prachů", "prachama", "keš",
        "kešu", "zlat*", "bohat*", "milion*", "miliard*", "diamant*", "šek", "bankovk*", "eura", "eur",
        "kačk*", "řetěz*",
        "kapse", "kapsy", "kapsu", "kapsách", "baller*", "swag", "kila", "kilo", "sázk*", "racky",
        "utrácím", "utrácet", "utrácíš", "utratím", "utratil*", "spend", "spending", "spent", "zbohat*", "bohatej",
    ]),
    (Mood::Love, &[
        "love", "loved", "lover", "loving", "heart", "hearts", "kiss*", "baby", "babe", "bae", "honey", "darling",
        "sweet*", "girl", "angel", "beautiful", "pretty", "hug", "hugs", "hugging", "touch*", "desire", "romance", "lásk*",
        "miluj*", "milá", "milý", "srdc*", "polib*", "pusa", "pusu", "zlatíčko", "kočk*", "holk*", "bejby",
        "bejbe", "krásn*", "objím*", "anděl*",
        "máma", "mámo", "mami", "mámu", "mom", "mommy", "home", "doma", "domů", "domov*", "spolu", "together", "kotě", "shawty",
        "babygirl", "kámošky", "kámoška", "kámošku", "kámošek", "kámoškou", "pusou", "pusinka", "pusinku", "pusinky",
        "=rád", "=ráda", "=rádi", "bába", "báby", "bábo", "babička", "babičk*", "babi", "grandma",
    ]),
    (Mood::Dark, &[
        "night", "dark*", "black", "shadow*", "death", "dead", "die", "dying", "ghost*", "alone", "lonely", "empty",
        "devil", "demon*", "hell", "grave", "blood*", "bleed*", "pain", "hurt*", "sad", "sadness", "broken", "lost",
        "noc", "noci", "nocí", "nocích", "tma", "tmy", "tmou", "tmě", "temn*", "černá", "černý", "černé", "černou",
        "cs:stín*", "smrt*", "mrtv*", "duch*", "peklo", "pekl*", "ďábel", "ďábl*", "démon*", "krev", "krve", "krví",
        "bolest*", "bolí", "sám", "sama", "samot*", "prázdn*", "smutn*", "smutek", "ztracen*", "hrob*",
        "tonight", "podsvětí", "underworld", "nightmare", "noční*",
        "nighttime", "umírám", "umírat", "umřu", "umřít", "umřel*", "zemřu", "zemřel*", "zdechnu", "zdechni", "zdechl*", "chcípnu", "chcíp*", "svinstv*", "hříchy", "hřích", "hříchů", "hříšn*", "sin", "sins", "sinner*",
        "lágr", "lágru", "lágrem", "zlo", "zla", "zle", "zlý", "zlá", "zlej", "evil", "nic", "nothing", "nikdo", "nobody",
    ]),
    (Mood::Wave, &[
        "water", "sea", "ocean", "wave*", "rain*", "dream*", "float*", "swim*", "river", "tears", "cry", "crying",
        "cried", "drown*", "flow*", "wine", "drunk", "liquor", "lean", "sip*", "pour*", "=moře", "=moři", "=mořem", "voda", "vodu",
        "vody", "vodě", "vlny", "vln*", "déšť", "dešt*", "prší", "sen", "sny", "snu", "snech", "sním", "plav*",
        "řeka", "řeky", "řece", "řekou", "slz*", "brečím", "brečíš", "brečet", "pláč*", "utop*", "opil*", "chlast*", "víno", "flašk*",
        "drink*", "pijem", "piju", "pijeme", "pití", "alkohol*", "drogy", "drog", "drug*", "weed", "kush", "tráva", "trávu", "trávy", "joint*", "hulím", "hulit", "zahulen*", "pill", "pills", "codeine", "molly",
        "pijeme", "pijou", "cs:pít", "pije", "pil", "pila", "pili", "chlastám", "chlastat", "ožral*", "ožrat", "nalej*", "panák*", "beer", "pivo", "piva", "pivem", "vodka", "vodku", "whiskey", "henny", "hennessy", "champagne", "šampus*", "šampaňsk*", "číše", "číši", "číší", "tipsy",
        "fet", "fetu", "feťák*", "fetuj*", "fetovat",
    ]),
    (Mood::Fast, &[
        "run", "running", "ran", "fast", "faster", "speed*", "race", "racing", "rush", "drive", "driving", "ride",
        "riding", "gas", "zoom", "quick*", "hurry", "chase*", "car", "whip", "lambo", "ferrari", "porsche",
        "benz", "jedu", "jedem", "jedeme", "jedeš", "rychl*", "běžím", "běží", "běžet", "běžíme", "běžel*", "utíkám", "utíká*", "závod*", "plyn*",
        "auto", "auta", "autem", "kára", "káru", "cs:fár*", "mašin*", "frčí*",
        "přidej", "nonstop", "nezastav*", "jeď", "běhám", "dostih*", "skrz",
        "hlavák", "hlaváku", "hlavákem",
    ]),
    (Mood::Stop, &[
        "stop", "wait", "enough", "never", "silence", "stůj", "počkej", "konec", "nikdy", "stačí",
        "přestaň", "ticho",
        "real", "realest", "true", "truth", "pravda", "pravdu", "pravdou", "pravá", "pravý", "pravej", "pravou", "legit",
    ]),
    (Mood::Grow, &[
        "big", "bigger", "huge", "giant", "massive", "world", "everything", "everybody", "forever", "always",
        "king", "queen", "boss", "legend*", "champion*", "velk*", "obrovsk*", "obří", "svět*", "všechno",
        "všichni", "navždy", "vždycky", "král*", "šéf",
        "život", "života", "životě", "životem", "life", "město", "města", "městě", "městem", "city", "všude", "everywhere",
        "já", "svět", "světy", "světa",
        "steroid*", "roidy", "anabolik*", "confident",
    ]),
    (Mood::Spin, &[
        "spin*", "round", "around", "turn*", "dizzy", "twist*", "roll*", "circle*", "točí*", "točím", "kolem",
        "dokola", "otáč*", "motá*", "kruh*",
        "hlava", "hlavě", "hlavu", "hlavou", "head", "mind", "motám",
        "kola", "kolo", "wheels", "rims",
    ]),
    (Mood::Flash, &[
        "light*", "shine", "shining", "bright", "flash*", "camera*", "sparkle*", "glow*", "neon", "světl*",
        "záři*", "blesk*", "svítí*", "svítím", "foť*", "fotk*",
        "oči", "očích", "očima", "eyes", "fresh", "new", "novej", "nový", "nová", "nové", "show", "barvy", "barev", "colors",
        "face", "faces", "ksicht", "ksichtu", "ksichte", "ksichtem", "xicht*", "obličej*", "gift", "gifted", "dárek", "dárky", "dárkem",
    ]),
    (Mood::Bounce, &[
        "bounce", "dance*", "tanc*", "tanč*", "party", "párty", "club", "klub*", "beat", "beats", "rap", "rapper*",
        "rapuj*", "track", "soundtrack", "album", "gang", "crew", "squad", "parta", "kluci", "boys", "bros",
        "bracho", "brácho", "bro", "kámo", "kámoš*", "cs:more", "cs:mores", "jump", "skáč*", "mosh", "pogo", "woodstock", "rave",
        "tracky", "tracku", "tracků", "trackem", "parket*", "dancefloor", "rappuju", "rappuj*", "rapuju", "talk", "talking", "mluv*", "kecy", "kecám", "kecáš", "kecá", "párty", "party",
        "kálo", "kála", "kálu", "kalba", "kalb*", "kalit", "kalíme", "kalím", "pařba", "pařb*", "pařit", "paříme", "moshpit", "broski", "mládež", "mladí", "mladej", "mladý", "young", "youngin", "youngsta",
    ]),
    (Mood::Slow, &[
        "zpomal*", "slow", "slowly", "pomalu", "slowmo", "slow-mo", "relax", "calm",
        "válo", "válím", "válíme", "válet", "jetlag",
    ]),
    (Mood::Freaky, &[
        "sex", "sexy", "sexu", "sexem", "freaky", "freak", "freaks", "hrátky", "hrátk*", "postel", "postele", "posteli",
        "postelí", "bed", "hotel", "hotelu", "hotelem", "hotely", "hotelích", "naked", "nahá", "nahý", "nahou",
        "tělo", "těla", "tělem", "těle", "body", "babygirl", "kunda", "kundy", "kundu", "kundo", "kundou", "pussy",
        "prdel", "prdeli", "prdelí", "zadek", "zadku", "ass", "booty", "hoe", "hoes", "děvka", "děvky", "děvko", "děvku",
        "děvkám", "děvkama", "whore", "thot", "thots", "milenk*", "lover",
        "mrdá*", "mrdat", "mrdal*", "šuk*", "kinky", "kalhotk*", "panties", "tanga", "nahatej", "nahatá", "nahatý", "naháč",
    ]),
    (Mood::Fake, &[
        "fake", "faker", "fakes", "fejk*", "faleš*", "falešn*", "krysa", "krysy", "krysu", "kryso", "krysou", "rat",
        "rats", "snitch*", "lie", "lies", "liar", "lying", "lež", "lži", "lžeš", "lhář*", "lháři", "lhal*", "glitch*",
        "smažk*", "smažím", "smažit", "cap", "capping", "cappin", "schíz*", "schýz*", "paranoi*", "paranoid", "error", "errors", "chyba", "chyby", "bug", "bugs", "lag", "konfident*", "krys*",
    ]),
    (Mood::Sorry, &[
        "sorry", "promiň", "promiňte", "pardon", "odpusť", "odpusťte", "odpuštění", "odpustit", "odpustíš",
        "forgive", "forgiven", "apologize", "lituju", "lituji",
    ]),
    (Mood::Split, &[
        "rozdělen*", "rozděl*", "rozpad*", "rozpůl*", "split", "divided", "apart", "napůl", "half", "rozchod*",
        "duo",
    ]),
    (Mood::Seek, &[
        "hledám", "hledáš", "hledá", "hledat", "hledáme", "hledal*", "search*", "looking", "seek*", "najdu", "najdeš",
        "najít", "najdem", "najdeme",
        "dívej", "dívám", "díváš", "dívat", "koukej", "koukám", "koukni", "cs:čum*", "sleduj*", "watch", "watching",
    ]),
    (Mood::Back, &[
        "zpět", "zpátky", "zpátk*", "back", "rewind", "návrat*", "vrátím", "vrátit", "vrátíš", "vrať", "vraťte",
    ]),
];

/// Small words that only hold a line together, set small and light.
const FILLER: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "so", "of", "to", "in", "on", "at", "for", "with", "by", "from",
    "is", "are", "was", "be", "i", "i'm", "im", "me", "my", "you", "your", "u", "it", "it's", "its", "this",
    "that", "we", "they", "he", "she", "his", "her", "do", "just", "like",
    "i", "v", "ve", "s", "se", "na", "do", "za", "z", "ze", "že", "je", "jsem", "jsi", "si", "to", "ta",
    "ten", "ty", "mi", "ti", "mě", "tě", "mu", "jak", "co", "ale", "tak", "už", "jen", "by", "aby",
    "k", "ke", "o", "u", "po", "pro", "mý", "můj", "tvůj", "tvý",
];

/// Ad-libs, which get thrown at the screen.
const SHOUTS: &[&str] = &[
    "yeah", "ey", "eyy", "hey", "yo", "oh", "ooh", "woo", "ah", "aha", "skrrt", "skrr", "brr", "brrr",
    "pow", "bang", "jo", "jou", "jau", "ha", "huh", "uh", "okay", "ok",
];

/// A stem matches words this many letters longer at most.
const STEM_ENDING: usize = 4;

/// A list entry, read once (see [`MOODS`] for how they're written).
pub(super) struct Entry {
    /// Folded, or as written for an exact one.
    text: String,
    stem: bool,
    exact: bool,
    /// Only in a Czech or Slovak song (true), or only in any other (false).
    czech: Option<bool>,
}

pub(super) fn prepare(list: &[&str]) -> Vec<Entry> {
    list.iter()
        .map(|entry| {
            let (entry, czech) = match (entry.strip_prefix("cs:"), entry.strip_prefix("en:")) {
                (Some(e), _) => (e, Some(true)),
                (_, Some(e)) => (e, Some(false)),
                _ => (*entry, None),
            };
            match entry.strip_prefix('=') {
                Some(exact) => Entry { text: exact.to_lowercase(), stem: false, exact: true, czech },
                None => {
                    let (text, stem) = entry.strip_suffix('*').map_or((entry, false), |s| (s, true));
                    Entry { text: icons::fold(text), stem, exact: false, czech }
                }
            }
        })
        .collect()
}

/// The word lists, read once.
struct Lists {
    moods: Vec<(Mood, Vec<Entry>)>,
    filler: Vec<Entry>,
    shouts: Vec<Entry>,
}

static LISTS: LazyLock<Lists> = LazyLock::new(|| Lists {
    moods: MOODS.iter().map(|(mood, words)| (*mood, prepare(words))).collect(),
    filler: prepare(FILLER),
    shouts: prepare(SHOUTS),
});

/// A lyric word, ready to be looked up.
pub(super) struct Token {
    /// Lowercase, diacritics kept.
    lower: String,
    /// Folded, then with English endings off: "burning" is also "burn",
    /// "racing" "race" and "runnin" "run".
    forms: Vec<String>,
    /// Sung in a Czech or Slovak song.
    czech: bool,
}

impl Token {
    pub(super) fn new(word: &str, czech: bool) -> Self {
        let folded = icons::fold(word);
        let mut forms = vec![folded.clone()];
        for end in ["s", "es", "'s", "ing", "in", "in'", "ed", "d"] {
            let Some(stem) = folded.strip_suffix(end).filter(|s| s.chars().count() >= 3) else {
                continue;
            };
            forms.push(stem.to_owned());
            forms.push(format!("{stem}e"));
            let mut back = stem.chars().rev();
            if let (Some(last), Some(before)) = (back.next(), back.next())
                && last == before
            {
                forms.push(stem[..stem.len() - last.len_utf8()].to_owned());
            }
        }
        Self { lower: word.to_lowercase(), forms, czech }
    }
}

/// Whether `token` is in a prepared list.
pub(super) fn listed(token: &Token, list: &[Entry]) -> bool {
    list.iter().any(|entry| {
        entry.czech.is_none_or(|czech| czech == token.czech)
            && if entry.exact {
                token.lower == entry.text
            } else if entry.stem {
                let max = entry.text.chars().count() + STEM_ENDING;
                token.forms.iter().any(|f| f.starts_with(&entry.text) && f.chars().count() <= max)
            } else {
                token.forms.contains(&entry.text)
            }
    })
}

fn mood(token: &Token) -> Option<Mood> {
    LISTS.moods.iter().find(|(_, words)| listed(token, words)).map(|(mood, _)| *mood)
}

/// Words only Czech and Slovak are full of, so their songs give themselves away.
const CZECH_TELLS: &[&str] = &[
    "je", "jsem", "jsi", "se", "si", "na", "ze", "ja", "ty", "mi", "jak", "co", "ale", "uz", "som", "sa",
    "ako", "nie", "neni", "mam", "kdyz", "jako", "jsou", "furt", "ted", "proc",
];

/// Whether the song is sung in Czech or Slovak.
fn czech(lyrics: &mv_enrich::Lyrics) -> bool {
    let (mut words, mut tells) = (0, 0);
    for line in &lyrics.synced {
        for word in line.text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
            words += 1;
            tells += CZECH_TELLS.contains(&icons::fold(word).as_str()) as usize;
        }
    }
    tells * 12 >= words.max(1)
}

/// One word of a line, with how it is sung.
struct Word {
    text: String,
    at: Duration,
    /// Seconds until the next word.
    held: f32,
    /// Size against an ordinary word.
    weight: f32,
    mood: Option<Mood>,
    /// Dressed its own way: a brand, a person, a word with a symbol.
    look: Option<&'static Look>,
    filler: bool,
    /// Shouted: in capitals, with an exclamation mark, or an ad-lib.
    loud: bool,
    seed: u64,
}

/// Line `i`'s words, weighed.
fn words_of(lyrics: &mv_enrich::Lyrics, i: usize) -> Vec<Word> {
    let line = &lyrics.synced[i];
    let starts = lyrics.word_starts(i);
    let line_end = lyrics.synced.get(i + 1).map(|l| l.at).or(line.end);
    // The chorus comes back, and hits harder than a verse.
    let key = |text: &str| icons::fold(text.trim());
    let this = key(&line.text);
    let chorus = lyrics.synced.iter().filter(|l| key(&l.text) == this).count() > 1;
    let count = starts.len();
    let czech = czech(lyrics);
    starts
        .iter()
        .enumerate()
        .filter_map(|(k, (range, at))| {
            let text: String = line.text.chars().skip(range.start).take(range.len()).collect();
            let bare = text.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
            if bare.is_empty() {
                return None;
            }
            let token = Token::new(bare, czech);
            let next = starts.get(k + 1).map(|(_, at)| *at).or(line_end);
            let held = next.map_or(0.5, |n| n.saturating_sub(*at).as_secs_f32()).min(2.0);
            let letters = bare.chars().filter(|c| c.is_alphanumeric()).count();
            let caps = letters >= 2 && bare.chars().all(|c| !c.is_lowercase());
            let shout = listed(&token, &LISTS.shouts);
            let loud = caps || text.contains('!') || shout;
            let filler = !loud && listed(&token, &LISTS.filler);
            let look = looks::look(&token);
            let mood = look
                .and_then(|l| l.mood)
                .or_else(|| mood(&token))
                .or(shout.then_some(Mood::Shake));

            let mut weight = if filler { 0.5 } else { 1.0 };
            weight += (letters as f32 - 4.0).clamp(0.0, 6.0) * 0.03;
            weight += 0.9 * remap(held, 0.35, 1.4);
            if loud {
                weight += 0.6;
            }
            if look.is_some() {
                weight += 0.55;
            } else if mood.is_some() {
                weight += 0.45;
            }
            if k + 1 == count && !filler {
                weight += 0.2;
            }
            if chorus {
                weight *= 1.12;
            }
            Some(Word {
                text: bare.to_owned(),
                at: *at,
                held,
                weight: weight.clamp(0.45, 2.6),
                mood,
                look,
                filler: filler && look.is_none(),
                loud,
                seed: (i as u64) << 20 | k as u64,
            })
        })
        .collect()
}

/// Whether a word is set heavy and comes in hard.
fn emphatic(word: &Word, hero: bool) -> bool {
    hero || word.weight >= 1.25 || word.loud || word.mood.is_some() || word.look.is_some()
}

/// A word laid out on the poster.
struct Placed {
    galley: Arc<Galley>,
    /// Its middle, before anything moves it.
    center: Pos2,
    /// Galley to screen.
    scale: f32,
    tilt: f32,
    hero: bool,
    emphatic: bool,
}

/// Lays line `words` out as a poster centered on `center`: each word as big
/// as its weight, the biggest (the hero) alone on its row, the rest packed
/// into rows, and the whole shrunk to fit `max` if it must.
fn lay_out(painter: &Painter, words: &[Word], center: Pos2, base: f32, max: Vec2) -> Vec<Placed> {
    // Rasters in three steps, so sizes don't fill the glyph atlas.
    let unit = ((base / 4.0).round() * 4.0).max(16.0);
    let raster = |px: f32| {
        [0.6, 1.2, 2.4]
            .map(|k| unit * k)
            .into_iter()
            .find(|r| *r >= px * 0.85)
            .unwrap_or(unit * 2.4)
    };
    let hero = words
        .iter()
        .enumerate()
        .filter(|(_, w)| !w.filler && w.weight >= 1.3)
        .max_by(|a, b| a.1.weight.total_cmp(&b.1.weight))
        .map(|(k, _)| k);

    struct Sized {
        galley: Arc<Galley>,
        scale: f32,
        /// The word's own width; `width` adds its symbol's room.
        word: f32,
        width: f32,
        height: f32,
    }
    let sized: Vec<Sized> = words
        .iter()
        .enumerate()
        .map(|(k, w)| {
            let emphatic = emphatic(w, Some(k) == hero);
            let mut px = base * w.weight * if Some(k) == hero { 1.45 } else { 1.0 };
            let family = match w.look {
                Some(look) => look.face.family(),
                None if emphatic => HEAVY,
                None if w.filler => REGULAR,
                None => BOLD,
            };
            let r = raster(px);
            let spacing = match w.look {
                Some(look) => look.spacing - 0.02,
                None if emphatic => -0.04,
                None => -0.02,
            };
            let format = TextFormat {
                font_id: FontId::new(r, FontFamily::Name(family.into())),
                extra_letter_spacing: spacing * r,
                ..Default::default()
            };
            let caps = w.loud || w.look.is_some_and(|l| l.caps);
            let text = if caps { w.text.to_uppercase() } else { w.text.clone() };
            let text = if w.look.is_some_and(|l| l.quotes) { format!("\u{201C}{text}\u{201D}") } else { text };
            let galley = painter.layout_job(LayoutJob::single_section(text, format));
            // A word alone too wide for the screen shrinks to fit it.
            px *= (max.x / (galley.size().x * px / r)).min(1.0);
            let scale = px / r;
            // Room for a symbol beside it.
            let symbol = if w.look.is_some_and(|l| l.symbol.is_some()) { SYMBOL_ROOM } else { 0.0 };
            Sized {
                word: galley.size().x * scale,
                width: (galley.size().x + galley.size().y * symbol) * scale,
                height: galley.size().y * scale,
                galley,
                scale,
            }
        })
        .collect();

    // Rows of word indexes, the hero alone on its own.
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut width = 0.0;
    for (k, s) in sized.iter().enumerate() {
        let gap = s.height * 0.3;
        let alone = Some(k) == hero || rows.last().is_some_and(|r| Some(r[0]) == hero);
        match rows.last_mut() {
            Some(row) if !alone && width + gap + s.width <= max.x => {
                row.push(k);
                width += gap + s.width;
            }
            _ => {
                rows.push(vec![k]);
                width = s.width;
            }
        }
    }
    // Set tight, like display type.
    let row_height = |row: &[usize]| row.iter().map(|&k| sized[k].height * 0.78).fold(0.0, f32::max);
    let total: f32 = rows.iter().map(|r| row_height(r)).sum();
    let fit = (max.y / total).min(1.0);

    let mut placed: Vec<Option<Placed>> = (0..words.len()).map(|_| None).collect();
    let mut y = center.y - total * fit / 2.0;
    for row in &rows {
        let height = row_height(row) * fit;
        let gaps: f32 = row.iter().skip(1).map(|&k| sized[k].height * 0.3).sum();
        let row_width = (row.iter().map(|&k| sized[k].width).sum::<f32>() + gaps) * fit;
        // Rows nudged off center a little, like they were set by hand.
        let nudge = wobble(words[row[0]].seed ^ 0x40b) * 0.05 * max.x * (rows.len() > 1) as u8 as f32;
        let mut x = center.x - row_width / 2.0 + nudge;
        for (j, &k) in row.iter().enumerate() {
            if j > 0 {
                x += sized[k].height * 0.3 * fit;
            }
            let s = &sized[k];
            let w = &words[k];
            let emphatic = emphatic(w, Some(k) == hero);
            placed[k] = Some(Placed {
                galley: Arc::clone(&s.galley),
                center: Pos2::new(x + s.word * fit / 2.0, y + height / 2.0),
                scale: s.scale * fit,
                tilt: if emphatic { wobble(w.seed ^ 0x7117) * 0.07 } else { 0.0 },
                hero: Some(k) == hero,
                emphatic,
            });
            x += s.width * fit;
        }
        y += height;
    }
    placed.into_iter().flatten().collect()
}

/// How a word moves at one moment.
struct Motion {
    /// From its place on the poster, in screen px.
    offset: Vec2,
    scale: f32,
    /// Stretch across and up, on top of `scale`.
    squash: Vec2,
    angle: f32,
    alpha: f32,
    /// The hit of a slammed word landing, 1 -> 0, which shakes the poster.
    impact: f32,
}

/// When a leaving word goes: once it has been sung, though not right away.
fn departure(word: &Word) -> f32 {
    (word.held + 0.15).clamp(0.6, 1.4)
}

/// How far a leaving word has gone, 0 -> 1 as it heads off screen.
fn carried(word: &Word, t: f32) -> f32 {
    let go = departure(word);
    ease_in_cubic(remap(t, go, go + 0.9))
}

/// The classic bounce of something dropped, 0 -> 1.
fn bounce(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

/// A word `t` seconds after it was sung, `h` px tall on screen, coming in
/// as its mood, or by how hard it is sung.
fn motion(word: &Word, emphatic: bool, t: f32, h: f32, width: f32) -> Motion {
    let mut m = Motion {
        offset: Vec2::ZERO,
        scale: 1.0,
        squash: Vec2::splat(1.0),
        angle: 0.0,
        alpha: remap(t, 0.0, 0.08),
        impact: 0.0,
    };
    // Slammed: falls onto the screen from big and hits it.
    let slam = |m: &mut Motion, hard: f32| {
        let k = remap(t, 0.0, 0.16);
        m.scale = 1.0 + 1.6 * hard * (1.0 - ease_in_cubic(k));
        m.alpha = remap(t, 0.0, 0.05);
        m.impact = if t >= 0.16 { hard * (1.0 - remap(t, 0.16, 0.5)) } else { 0.0 };
    };
    let pop = |m: &mut Motion| {
        m.scale = 0.5 + 0.5 * spring(remap(t, 0.0, 0.4));
        m.offset.y = (1.0 - ease_out_cubic(remap(t, 0.0, 0.25))) * 0.3 * h;
    };
    match word.mood {
        Some(Mood::Shake | Mood::Stop) => slam(&mut m, 1.0),
        Some(Mood::Fall) => m.offset.y = -(1.0 - bounce(remap(t, 0.0, 0.55))) * 1.6 * h,
        Some(Mood::Rise) => {
            let k = ease_out_cubic(remap(t, 0.0, 0.4));
            m.offset.y = (1.0 - k) * 1.6 * h - t.min(6.0) * 0.18 * h;
            m.squash.y = 1.0 + 0.4 * (1.0 - k);
            m.alpha = remap(t, 0.0, 0.1);
        }
        Some(Mood::Money) => {
            // Spins in like a coin.
            let k = ease_out_cubic(remap(t, 0.0, 0.5));
            m.squash.x = ((1.0 - k) * TAU).cos().abs().max(0.05);
            m.scale = 0.7 + 0.3 * k;
        }
        Some(Mood::Cold) => {
            m.alpha = remap(t, 0.0, 0.45);
            m.scale = 1.15 - 0.15 * ease_out_cubic(remap(t, 0.0, 0.6));
        }
        Some(Mood::Dark) => m.alpha = 1.0,
        Some(Mood::Fast) => {
            let k = ease_out_cubic(remap(t, 0.0, 0.22));
            m.offset.x = -(1.0 - k) * width * 0.5;
            m.squash = Vec2::new(1.0 + 0.8 * (1.0 - k), 1.0 - 0.25 * (1.0 - k));
        }
        Some(Mood::Grow) => {
            // Comes small, then keeps on growing past its place.
            m.alpha = remap(t, 0.0, 0.15);
            m.scale = (0.3 + 0.7 * ease_out_cubic(remap(t, 0.0, 0.6))) * (1.0 + 0.15 * remap(t, 0.6, 3.0));
        }
        Some(Mood::Spin) => {
            let k = ease_out_cubic(remap(t, 0.0, 0.55));
            m.angle = -(1.0 - k) * PI * 1.5;
            m.scale = 0.5 + 0.5 * k;
        }
        Some(Mood::Flash) => slam(&mut m, 0.6),
        Some(Mood::Slow) => {
            let k = ease_out_cubic(remap(t, 0.0, 1.2));
            m.alpha = remap(t, 0.0, 0.9);
            m.squash.x = 1.3 - 0.3 * k;
            m.scale = 1.06 - 0.06 * k;
        }
        Some(Mood::Bounce | Mood::Freaky | Mood::Fake | Mood::Split | Mood::Seek) => pop(&mut m),
        Some(Mood::Sorry) => {
            // Bows its head.
            let k = ease_out_cubic(remap(t, 0.2, 1.0));
            m.alpha = remap(t, 0.0, 0.5);
            m.scale = 1.0 - 0.12 * k;
            m.offset.y = 0.15 * h * k;
            m.angle = 0.06 * k;
        }
        Some(Mood::Back) => {
            let k = ease_out_cubic(remap(t, 0.0, 0.35));
            m.offset.x = (1.0 - k) * width * 0.5;
            m.squash = Vec2::new(1.0 + 0.6 * (1.0 - k), 1.0 - 0.2 * (1.0 - k));
        }
        Some(mood @ (Mood::FlyAway | Mood::DriveAway | Mood::Away)) => {
            pop(&mut m);
            let gone = carried(word, t);
            m.offset.x += gone * width * 0.8;
            match mood {
                Mood::FlyAway => {
                    m.offset.y -= gone * width * 0.3;
                    m.angle -= 0.35 * gone;
                }
                Mood::Away => m.alpha *= 1.0 - gone,
                _ => {}
            }
        }
        Some(Mood::Fire | Mood::Love | Mood::Wave) => pop(&mut m),
        None if word.loud => slam(&mut m, 1.0),
        None if emphatic => slam(&mut m, 0.5),
        None if word.filler => {
            m.alpha = remap(t, 0.0, 0.2);
            m.offset.y = (1.0 - ease_out_cubic(remap(t, 0.0, 0.3))) * 0.25 * h;
        }
        None => pop(&mut m),
    }
    m
}

/// A word's glyph `j` of `n`, `t` seconds after the word was sung, at song
/// time `clock`: how far it moves, in galley heights, and how visible it is.
fn glyph_motion(word: &Word, j: usize, n: usize, t: f32, clock: f32, groove: Groove) -> (Vec2, f32) {
    let j = j as f32;
    let noise = |k: u64| wobble(word.seed ^ (j as u64) << 8 ^ k);
    // A held note spreads its letters while it lasts.
    let held = if word.held > 0.7 {
        (j - (n as f32 - 1.0) / 2.0) * 0.07 * remap(t, 0.15, word.held)
    } else {
        0.0
    };
    let mut offset = Vec2::new(held, 0.0);
    let mut alpha = 1.0;
    match word.mood {
        Some(Mood::Fall) => {
            // Letters drop one after another.
            let k = remap(t - j * 0.03, 0.0, 0.5);
            offset.y -= (1.0 - bounce(k)) * 0.8;
        }
        Some(Mood::Rise) => offset.y += (1.0 - ease_out_cubic(remap(t - j * 0.04, 0.0, 0.5))) * 0.7,
        Some(Mood::Fire) => {
            let flicker = (clock * 13.0 + j * 1.7).sin().abs();
            offset.y -= flicker * 0.06 * (0.5 + groove.beat);
        }
        Some(Mood::Shake) => {
            // New jitter 24 times a second, harder on the beat.
            let tick = (clock * 24.0) as u64;
            offset += Vec2::new(noise(tick), noise(tick ^ 0xffff)) * 0.05 * (0.6 + groove.beat);
        }
        Some(Mood::Cold) => offset += Vec2::new(noise(1), noise(2)) * 0.02,
        Some(Mood::Wave) => offset.y += (clock * 4.0 - j * 0.6).sin() * 0.08,
        Some(Mood::Dark) => alpha = remap(t - j * 0.06, 0.0, 0.18),
        Some(Mood::Freaky) => offset += Vec2::new((clock * 3.0 + j * 0.5).sin() * 0.05, (clock * 6.0 + j * 0.5).sin() * 0.02),
        Some(Mood::Fake) => {
            // Now and then a letter drops out or jumps.
            let tick = (clock * 12.0) as u64;
            if noise(tick) > 0.55 {
                alpha = 0.2;
            }
            if noise(tick ^ 0xfa4e) > 0.7 {
                offset.x += noise(tick ^ 0x1) * 0.08;
            }
        }
        Some(Mood::Sorry) => offset.y += (clock * 2.0 + j * 0.7).sin().abs() * 0.02,
        Some(Mood::Split) => {
            let side = (j - (n as f32 - 1.0) / 2.0).signum();
            offset.x += side * 0.25 * ease_out_cubic(remap(t, 0.25, 0.9));
        }
        Some(Mood::Seek) => alpha = remap(t - j * 0.05, 0.0, 0.12),
        Some(Mood::Bounce) => {
            // Every other letter hops, the rest land, and the beat throws them all up.
            let hop = (clock * 7.0 + j * 0.9).sin().abs();
            let side = if (j as usize).is_multiple_of(2) { 1.0 } else { 0.6 };
            offset.y -= hop * 0.07 * side + groove.beat * 0.1 * side;
        }
        _ => {}
    }
    (offset, alpha)
}

/// A word's color by its mood, glyph `j` at song time `clock`, or `base`.
fn mood_color(mood: Option<Mood>, base: Color32, j: usize, clock: f32) -> Color32 {
    match mood {
        Some(Mood::Fire) => {
            let heat = 0.5 + 0.5 * (clock * 9.0 + j as f32 * 2.3).sin();
            lerp_color(Color32::from_rgb(255, 150, 40), Color32::from_rgb(255, 50, 30), heat)
        }
        Some(Mood::Cold) => Color32::from_rgb(175, 228, 255),
        Some(Mood::Money) => {
            // A shine running across.
            let shine = (1.0 - ((clock * 0.8).fract() * 1.6 - j as f32 * 0.08).abs() * 6.0).max(0.0);
            lerp_color(Color32::from_rgb(255, 200, 60), Color32::WHITE, shine)
        }
        Some(Mood::Love) => Color32::from_rgb(255, 85, 125),
        Some(Mood::Dark) => lerp_color(base, Color32::BLACK, 0.35),
        Some(Mood::Freaky) => {
            lerp_color(Color32::from_rgb(255, 70, 170), Color32::from_rgb(170, 90, 255), (clock * 1.5 + j as f32 * 0.4).sin() * 0.5 + 0.5)
        }
        Some(Mood::Sorry) => lerp_color(base, Color32::from_gray(150), 0.4),
        Some(Mood::Flash) => {
            // A strobe that settles into a pale glow.
            let strobe = if (clock * 18.0).sin() > 0.3 { 1.0 } else { 0.55 };
            lerp_color(base, Color32::WHITE, strobe)
        }
        _ => base,
    }
}

/// Adds `galley`'s glyphs to `mesh`: glyph `j` of `n` moved by `glyph(j, n)`
/// (galley px) and colored by it, then all placed by `place` from galley
/// coordinates centered on its middle. `atlas` is the font atlas size in texels.
fn glyph_mesh(
    mesh: &mut Mesh,
    galley: &Galley,
    atlas: Vec2,
    place: impl Fn(Vec2) -> Pos2,
    glyph: impl Fn(usize, usize) -> (Vec2, Color32),
) {
    let middle = galley.size() / 2.0;
    // Glyphs come as quads of four vertices.
    let n = galley.rows.iter().map(|r| r.visuals.mesh.vertices.len() / 4).sum();
    let mut first = 0;
    for row in &galley.rows {
        let glyphs = &row.visuals.mesh;
        let base = mesh.vertices.len() as u32;
        mesh.indices.extend(glyphs.indices.iter().map(|i| base + i));
        mesh.vertices.extend(glyphs.vertices.iter().enumerate().map(|(k, v)| {
            let (offset, color) = glyph(first + k / 4, n);
            Vertex {
                pos: place(row.pos.to_vec2() + v.pos.to_vec2() - middle + offset),
                uv: Pos2::new(v.uv.x / atlas.x, v.uv.y / atlas.y),
                color,
            }
        }));
        first += glyphs.vertices.len() / 4;
    }
}

/// A look's symbol beside its word, centered on `at`, about as tall as the
/// word (`h`): it springs up `t` seconds after the word is sung, sways and
/// beats along.
#[allow(clippy::too_many_arguments)]
fn symbol_shape(
    painter: &Painter,
    atlas: Vec2,
    symbol: char,
    look: &Look,
    at: Pos2,
    h: f32,
    t: f32,
    clock: f32,
    groove: Groove,
    alpha: f32,
) -> Shape {
    /// One raster for every symbol, however big it's drawn.
    const RASTER: f32 = 160.0;
    let font = FontId::new(RASTER, FontFamily::Name(looks::SYMBOLS.into()));
    let galley = painter.layout_no_wrap(symbol.to_string(), font, Color32::WHITE);
    let grow = spring(remap(t, 0.0, 0.6)) * (1.0 + 0.08 * groove.beat);
    let scale = h * 0.95 / galley.size().y * grow;
    let turn = Rot2::from_angle((clock * 0.9).sin() * 0.12);
    let color = lerp_color(look.fill.main(), Color32::WHITE, 0.2).gamma_multiply(0.95 * alpha);
    let mut mesh = Mesh::default();
    glyph_mesh(&mut mesh, &galley, atlas, |v| at + turn * (v * scale), |_, _| (Vec2::ZERO, color));
    Shape::mesh(mesh)
}

/// The plane or car coming for a leaving word (see [`departure`]): it rides
/// in from the left to the word's back, then off with it, pushing it ahead.
#[allow(clippy::too_many_arguments)]
fn vehicle(
    painter: &Painter,
    atlas: Vec2,
    word: &Word,
    spot: &Placed,
    m: &Motion,
    t: f32,
    h: f32,
    width: f32,
) -> Option<Shape> {
    /// One raster for both, however big they're drawn.
    const RASTER: f32 = 128.0;
    let (icon, size, turn) = match word.mood? {
        // Phosphor's plane points up and to the right.
        Mood::FlyAway => ('\u{e5d6}', 1.3, std::f32::consts::FRAC_PI_4),
        Mood::DriveAway => ('\u{e8cc}', 1.2, 0.0),
        _ => return None,
    };
    let go = departure(word);
    let coming = ease_out_cubic(remap(t, go - 0.45, go));
    let alpha = remap(t, go - 0.45, go - 0.3) * m.alpha;
    if alpha <= 0.0 {
        return None;
    }
    let galley = painter.layout_no_wrap(icon.to_string(), FontId::new(RASTER, FontFamily::Name(icons::FONT.into())), Color32::WHITE);
    let scale = h * size / galley.size().y;
    // Nose to the word's tail, coming from off screen; then wherever the word goes.
    let back = spot.galley.size().x * spot.scale * m.scale / 2.0 + galley.size().x * scale * 0.45;
    let behind = Rot2::from_angle(spot.tilt + m.angle) * Vec2::new(-back, 0.0);
    let at = spot.center + m.offset + behind - Vec2::new((1.0 - coming) * width * 0.7, 0.0);
    // A plane comes in level, so it banks back from its glyph's climb; and bobs.
    let bob = Vec2::new(0.0, (t * 9.0).sin() * 0.04 * h);
    let rotation = Rot2::from_angle(turn + spot.tilt + m.angle);
    let mut mesh = Mesh::default();
    let color = Color32::WHITE.gamma_multiply(alpha);
    glyph_mesh(&mut mesh, &galley, atlas, |v| at + bob + rotation * (v * scale), |_, _| (Vec2::ZERO, color));
    Some(Shape::mesh(mesh))
}

/// The ink of a galley, centered on its middle.
fn ink(galley: &Galley) -> Rect {
    galley
        .rows
        .iter()
        .map(|r| r.visuals.mesh_bounds.translate(r.pos.to_vec2()))
        .fold(Rect::NOTHING, Rect::union)
        .translate(-galley.size() / 2.0)
}

/// Lyrics that live (see the module): the line being sung built up as a
/// poster word by word, the last one blowing apart as it begins. `palette`
/// is the cover's; over a music video (`video`) plain words turn white.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_live(
    painter: &Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    palette: [[f32; 3]; 3],
    video: f32,
    groove: Groove,
) {
    let Some(now) = lyrics.line_at(position) else {
        return;
    };
    let clock = position.as_secs_f32();
    let base = size * LIVE_SCALE;
    let center = Pos2::new(rect.center().x, center_y);
    // Narrow enough that a line stacks up into rows, like a poster.
    let max = Vec2::new((rect.width() * 0.86).min(base * 9.0), base * LIVE_HEIGHT);
    let atlas = Vec2::from(painter.fonts(|f| f.font_image_size()).map(|v| v as f32));
    let (light, deep) = word_colors(palette[0]);
    let (accent, _) = word_colors(palette[1]);
    let light = lerp_color(light, Color32::WHITE, video);
    let accent = lerp_color(accent, Color32::WHITE, video * 0.5);

    // The last line, blowing apart as this one starts.
    let since_line = (position - lyrics.synced[now].at).as_secs_f32();
    let leaving = now.checked_sub(1).filter(|_| since_line < LIVE_EXIT);
    let lines = leaving.map(|i| (i, remap(since_line, 0.0, LIVE_EXIT))).into_iter().chain([(now, 0.0)]);

    let mut shapes = Vec::new();
    let mut shake = 0.0f32;
    for (i, exit) in lines {
        let words = words_of(lyrics, i);
        if words.is_empty() {
            continue;
        }
        let placed = lay_out(painter, &words, center, base, max);
        let latest = words.iter().rposition(|w| w.at <= position);
        for (k, (word, spot)) in words.iter().zip(&placed).enumerate() {
            let t = position.saturating_sub(word.at).as_secs_f32();
            if word.at > position {
                continue;
            }
            let h = spot.galley.size().y * spot.scale;
            let mut m = motion(word, spot.emphatic, t, h, rect.width());
            if exit == 0.0 {
                shake = shake.max(m.impact);
            }
            // The word being sung punches on the beat; the hero breathes with the bass.
            if Some(k) == latest {
                m.scale *= 1.0 + 0.1 * groove.beat;
            }
            if spot.hero {
                m.scale *= 1.0 + 0.04 * groove.bass;
            }
            // Blown away: each word flies off its own way, growing as it fades.
            let exit = ease_out_cubic(exit);
            m.offset += Vec2::angled(wobble(word.seed ^ 0xb10) * PI) * exit * 1.4 * h;
            m.scale *= 1.0 + 0.6 * exit;
            m.angle += wobble(word.seed ^ 0x5017) * exit * 0.6;
            m.alpha *= 1.0 - exit;
            if m.alpha <= 0.0 {
                continue;
            }

            let turn = Rot2::from_angle(spot.tilt + m.angle);
            let scale = spot.scale * m.scale;
            let at = spot.center + m.offset;
            let slant = word.look.map_or(0.0, |l| l.slant);
            let place = |v: Vec2| {
                let v = Vec2::new(v.x - v.y * slant, v.y);
                at + turn * (Vec2::new(v.x * m.squash.x, v.y * m.squash.y) * scale)
            };
            let gh = spot.galley.size().y;
            let n_glyphs = word.text.chars().count();

            // The hero word on a sticker in the cover's deep tone.
            let base_color = if spot.hero {
                Color32::WHITE
            } else if spot.emphatic {
                if wobble(word.seed ^ 0xc01) > 0.0 { light } else { accent }
            } else {
                lerp_color(light, Color32::WHITE, 0.4)
            };
            let fade = if word.filler { 0.75 } else { 1.0 };
            let quad = |r: Rect, color: Color32| {
                let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
                Shape::convex_polygon(corners.map(|c| place(c.to_vec2())).to_vec(), color, Stroke::NONE)
            };
            if let Some(look) = word.look {
                if let Some(symbol) = look.symbol {
                    let beside = place(Vec2::new(spot.galley.size().x / 2.0 + gh * SYMBOL_ROOM / 2.0, 0.0));
                    shapes.push(symbol_shape(painter, atlas, symbol, look, beside, h * m.scale, t, clock, groove, m.alpha));
                }
                if let Some(plate) = look.plate {
                    shapes.push(quad(ink(&spot.galley).expand2(Vec2::new(0.14, 0.06) * gh), plate.gamma_multiply(m.alpha)));
                }
                // Stripes under the word grow out from its middle.
                let r = ink(&spot.galley);
                let grow = ease_out_cubic(remap(t, 0.05, 0.4));
                let band = 0.075 * gh;
                for (n, color) in look.stripes.iter().enumerate() {
                    let top = r.bottom() + 0.06 * gh + n as f32 * band;
                    let half = r.width() / 2.0 * grow;
                    let stripe = Rect::from_min_max(Pos2::new(r.center().x - half, top), Pos2::new(r.center().x + half, top + band * 0.8));
                    shapes.push(quad(stripe, color.gamma_multiply(m.alpha)));
                }
            } else if spot.hero && word.mood.is_none() {
                let r = ink(&spot.galley).expand2(Vec2::new(0.12, 0.04) * gh);
                shapes.push(quad(r, deep.gamma_multiply(m.alpha)));
            }

            // A plane or a car comes for a leaving word and takes it away.
            if let Some(vehicle) = vehicle(painter, atlas, word, spot, &m, t, h, rect.width()) {
                shapes.push(vehicle);
            }

            // Fast words leave a smear behind them as they come.
            if word.mood == Some(Mood::Fast) && t < 0.3 {
                for (s, back) in [0.25f32, 0.5, 0.75].into_iter().enumerate() {
                    let trail = |v: Vec2| place(v) - Vec2::new(back * 0.6 * h * (1.0 - t / 0.3), 0.0);
                    let tint = base_color.gamma_multiply(m.alpha * 0.25 / (s as f32 + 1.0));
                    let mut mesh = Mesh::default();
                    glyph_mesh(&mut mesh, &spot.galley, atlas, trail, |_, _| (Vec2::ZERO, tint));
                    shapes.push(Shape::mesh(mesh));
                }
            }

            // Hit hard, or on a strong beat, loud words split into red and cyan.
            let split = m.impact.max(if spot.emphatic { (groove.beat - 0.45) * 1.6 } else { 0.0 }).clamp(0.0, 1.0);
            let glyph = |j: usize, n: usize, color: Color32| {
                let (offset, alpha) = glyph_motion(word, j, n.max(n_glyphs), t, clock, groove);
                (offset * gh, color.gamma_multiply(alpha))
            };
            if split > 0.0 {
                for (side, tint) in [(-1.0, Color32::from_rgb(255, 40, 90)), (1.0, Color32::from_rgb(40, 220, 255))] {
                    let shifted = |v: Vec2| place(v) + Vec2::new(side * split * 0.06 * h, 0.0);
                    let tint = tint.gamma_multiply(m.alpha * split * 0.7);
                    let mut mesh = Mesh::default();
                    glyph_mesh(&mut mesh, &spot.galley, atlas, shifted, |j, n| glyph(j, n, tint));
                    shapes.push(Shape::mesh(mesh));
                }
            }
            let mut mesh = Mesh::default();
            glyph_mesh(&mut mesh, &spot.galley, atlas, place, |j, n| {
                let color = match word.look {
                    Some(look) => look.fill.color(j, n, clock),
                    None => mood_color(word.mood, base_color, j, clock),
                };
                glyph(j, n, color.gamma_multiply(m.alpha * fade))
            });
            shapes.push(Shape::mesh(mesh));
        }
    }

    // A slammed word shakes the whole poster, and the beat swells it.
    let tick = (clock * 30.0) as u64;
    let jolt = Vec2::new(wobble(tick), wobble(tick ^ 0x5a5a)) * shake * 0.05 * base;
    let swell = 1.0 + 0.035 * groove.beat;
    let transform = |p: Pos2| center + (p - center) * swell + jolt;
    for shape in &mut shapes {
        if let Shape::Mesh(mesh) = shape {
            for v in &mut Arc::make_mut(mesh).vertices {
                v.pos = transform(v.pos);
            }
        } else if let Shape::Path(path) = shape {
            for p in &mut path.points {
                *p = transform(*p);
            }
        }
    }
    painter.extend(shapes);
}

/// How far the poster may reach above and below its middle, for the shade
/// behind it over a music video.
pub(super) fn reach(size: f32) -> f32 {
    size * LIVE_SCALE * LIVE_HEIGHT * 0.8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word's mood in an English song, or in a Czech one.
    fn en(word: &str) -> Option<Mood> {
        mood(&Token::new(word, false))
    }
    fn cs(word: &str) -> Option<Mood> {
        mood(&Token::new(word, true))
    }

    #[test]
    fn words_find_their_mood() {
        assert_eq!(cs("Hoří"), Some(Mood::Fire));
        assert_eq!(en("fire"), Some(Mood::Fire));
        assert_eq!(cs("PENÍZE"), Some(Mood::Money));
        assert_eq!(en("falling"), Some(Mood::Fall));
        assert_eq!(cs("srdcem"), Some(Mood::Love));
        assert_eq!(en("table"), None);
        // A look's own way of moving wins over its word's mood.
        assert_eq!(looks::look(&Token::new("nike", false)).and_then(|l| l.mood), Some(Mood::Fast));
        // English word forms, and the newer moods.
        assert_eq!(en("burning"), Some(Mood::Fire));
        assert_eq!(en("cars"), Some(Mood::Fast));
        assert_eq!(en("runnin"), Some(Mood::Fast));
        assert_eq!(cs("světě"), Some(Mood::Grow));
        assert_eq!(en("lights"), Some(Mood::Flash));
        assert_eq!(cs("točí"), Some(Mood::Spin));
        assert_eq!(cs("slzy"), Some(Mood::Wave));
        assert_eq!(en("bounce"), Some(Mood::Bounce));
        assert_eq!(cs("zpomal"), Some(Mood::Slow));
        assert_eq!(cs("životem"), Some(Mood::Grow));
        // Everyday words folding into listed ones stay plain.
        for word in ["more", "rekni", "bez", "dost", "student", "stink", "far", "huge", "pillow"] {
            assert_ne!(en(word).filter(|m| *m != Mood::Grow), Some(Mood::Wave), "{word}");
        }
        assert_eq!(en("more"), None);
        assert_eq!(en("stink"), None);
        assert_eq!(en("far"), None);
        assert_eq!(en("student"), None);
        assert_eq!(en("huge"), Some(Mood::Grow));
        // "More" is a word, "more" in a Czech song is "bro", and "moře" is the sea.
        assert_eq!(cs("more"), Some(Mood::Bounce));
        assert_eq!(cs("moře"), Some(Mood::Wave));
        assert_eq!(en("moře"), Some(Mood::Wave));
        assert_eq!(cs("led"), Some(Mood::Cold));
        // Leaving, whichever way.
        assert_eq!(cs("odletím"), Some(Mood::FlyAway));
        assert_eq!(cs("uletěl"), Some(Mood::FlyAway));
        assert_eq!(cs("odjedu"), Some(Mood::DriveAway));
        assert_eq!(en("skrrt"), Some(Mood::DriveAway));
        assert_eq!(cs("odejdu"), Some(Mood::Away));
        assert_eq!(cs("pryč"), Some(Mood::Away));
        assert_eq!(cs("letím"), Some(Mood::Rise));
        assert_eq!(cs("letíš"), Some(Mood::Rise));
        assert_eq!(cs("pijeme"), Some(Mood::Wave));
        assert_eq!(cs("posteli"), Some(Mood::Freaky));
        assert_eq!(cs("krysa"), Some(Mood::Fake));
        assert_eq!(cs("promiň"), Some(Mood::Sorry));
        assert_eq!(cs("rozdělený"), Some(Mood::Split));
        assert_eq!(cs("hledáš"), Some(Mood::Seek));
        assert_eq!(cs("zpátky"), Some(Mood::Back));
        assert_eq!(cs("já"), Some(Mood::Grow));
        assert_eq!(cs("pravá"), Some(Mood::Stop));
        assert_eq!(cs("smažky"), Some(Mood::Fake));
        assert_eq!(cs("kálo"), Some(Mood::Bounce));
        assert_eq!(cs("vypadnu"), Some(Mood::Away));
        assert_eq!(cs("ráda"), Some(Mood::Love));
        // "Rada" is advice, "lip" a lip, "cum" English.
        assert_eq!(cs("rada"), None);
        assert_eq!(en("lip"), None);
        assert_eq!(en("cum"), None);
        assert_eq!(en("led"), None);
        // Stems don't reach far past themselves.
        assert_eq!(en("burningmanfestival"), None);
    }

    #[test]
    fn bounce_lands() {
        assert_eq!(bounce(0.0), 0.0);
        assert!((bounce(1.0) - 1.0).abs() < 1e-4);
    }
}
