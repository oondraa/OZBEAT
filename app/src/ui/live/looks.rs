//! Names dressed their own way in the live lyrics: brands in their house
//! style (type, colors, stripes; never their logos), people each with a look
//! of their own, and words that bring a symbol along ("ankh" ☥).
//!
//! A new name is one more row in [`LOOKS`]. Names are matched like the mood
//! words: folded, `*` for stems, English endings off by themselves.

use eframe::egui::Color32;

use std::sync::LazyLock;

use super::{Entry, Mood, Token, lerp_color, listed, prepare};

/// Typefaces a look can be set in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Face {
    Heavy,
    /// High-contrast serif, for fashion houses.
    Serif,
    /// Blackletter.
    Gothic,
    /// Extra black grotesque.
    Black,
    Mono,
}

impl Face {
    pub(super) fn family(self) -> &'static str {
        match self {
            Face::Heavy => super::HEAVY,
            Face::Serif => SERIF,
            Face::Gothic => GOTHIC,
            Face::Black => BLACK,
            Face::Mono => MONO,
        }
    }
}

const SERIF: &str = "live-serif";
const GOTHIC: &str = "live-gothic";
const BLACK: &str = "live-black";
const MONO: &str = "live-mono";
/// Font family of the symbols behind words.
pub(super) const SYMBOLS: &str = "live-symbols";

/// The faces from the system, first found wins; without any, a face falls
/// back to the heavy one.
pub(crate) const FACES: &[(&str, &[&str])] = &[
    (SERIF, &[
        "C:\\Windows\\Fonts\\BOD_B.TTF",
        "C:\\Windows\\Fonts\\georgiab.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf",
    ]),
    (GOTHIC, &["C:\\Windows\\Fonts\\OLDENGL.TTF", "C:\\Windows\\Fonts\\BOD_B.TTF", "C:\\Windows\\Fonts\\georgiab.ttf"]),
    (BLACK, &["C:\\Windows\\Fonts\\ariblk.ttf", "C:\\Windows\\Fonts\\impact.ttf"]),
    (MONO, &["C:\\Windows\\Fonts\\consolab.ttf", "C:\\Windows\\Fonts\\courbd.ttf"]),
    (SYMBOLS, &[
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]),
];

/// How a look colors its letters.
#[derive(Clone, Copy, Debug)]
pub(super) enum Fill {
    Solid(Color32),
    /// Letter by letter, round and round.
    Letters(&'static [Color32]),
    /// From the first letter's color to the last's.
    Fade(Color32, Color32),
    /// Polished metal of this tone, a shine running across.
    Chrome(Color32),
    /// The first letter in one color, the rest in another.
    Initial(Color32, Color32),
}

impl Fill {
    /// Letter `j` of `n` at song time `clock`.
    pub(super) fn color(self, j: usize, n: usize, clock: f32) -> Color32 {
        match self {
            Fill::Solid(c) => c,
            Fill::Letters(colors) => colors[j % colors.len()],
            Fill::Fade(from, to) => lerp_color(from, to, j as f32 / (n.max(2) - 1) as f32),
            Fill::Chrome(tone) => {
                let shine = (1.0 - ((clock * 0.7).fract() * 1.8 - j as f32 * 0.09).abs() * 5.0).max(0.0);
                lerp_color(tone, Color32::WHITE, 0.25 + 0.75 * shine)
            }
            Fill::Initial(first, rest) => {
                if j == 0 {
                    first
                } else {
                    rest
                }
            }
        }
    }

    /// The color the look reads as, for what goes with it.
    pub(super) fn main(self) -> Color32 {
        match self {
            Fill::Solid(c) | Fill::Fade(c, _) | Fill::Chrome(c) | Fill::Initial(c, _) => c,
            Fill::Letters(colors) => colors[0],
        }
    }
}

/// How a name is dressed.
#[derive(Clone, Copy, Debug)]
pub(super) struct Look {
    pub face: Face,
    pub fill: Fill,
    /// A plate behind the word.
    pub plate: Option<Color32>,
    /// Bands under the word, top to bottom.
    pub stripes: &'static [Color32],
    /// Extra space between letters, in font sizes.
    pub spacing: f32,
    /// Lean to the right, as a shear.
    pub slant: f32,
    pub caps: bool,
    /// In "quotation marks".
    pub quotes: bool,
    /// A big symbol behind the word.
    pub symbol: Option<char>,
    /// How it moves, as a mood word would.
    pub mood: Option<Mood>,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const WHITE: Color32 = rgb(255, 255, 255);
const INK: Color32 = rgb(12, 12, 12);
const GOLD: Color32 = rgb(222, 184, 92);
const SILVER: Color32 = rgb(176, 184, 196);

const PLAIN: Look = Look {
    face: Face::Heavy,
    fill: Fill::Solid(WHITE),
    plate: None,
    stripes: &[],
    spacing: 0.0,
    slant: 0.0,
    caps: false,
    quotes: false,
    symbol: None,
    mood: None,
};

/// Names and their looks; the first listing wins.
const LOOKS: &[(&[&str], Look)] = &[
    // Fashion and brands, in their house colors and type.
    (&["gucci"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), stripes: &[rgb(0, 112, 60), rgb(200, 16, 46), rgb(0, 112, 60)], spacing: 0.06, caps: true, ..PLAIN }),
    (&["prada"], Look { face: Face::Serif, plate: Some(INK), spacing: 0.14, caps: true, ..PLAIN }),
    (&["bape", "bapesta"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(96, 124, 66), rgb(150, 160, 90), rgb(66, 82, 48), rgb(186, 176, 120)]), caps: true, ..PLAIN }),
    (&["supreme"], Look { plate: Some(rgb(218, 41, 28)), slant: 0.2, ..PLAIN }),
    (&["louis", "vuitton", "lv"], Look { face: Face::Serif, fill: Fill::Fade(rgb(214, 170, 100), rgb(120, 82, 44)), spacing: 0.1, caps: true, ..PLAIN }),
    (&["balenciaga", "balenci"], Look { face: Face::Black, fill: Fill::Solid(INK), plate: Some(WHITE), spacing: -0.02, caps: true, ..PLAIN }),
    (&["offwhite", "off-white", "virgil", "abloh"], Look { face: Face::Mono, caps: true, quotes: true, ..PLAIN }),
    (&["nike", "nikes", "airmax", "airforce", "af1"], Look { face: Face::Black, fill: Fill::Solid(rgb(241, 90, 36)), slant: 0.2, caps: true, mood: Some(Mood::Fast), ..PLAIN }),
    (&["adidas"], Look { stripes: &[WHITE, WHITE, WHITE], ..PLAIN }),
    (&["jordan", "jordans", "jordany"], Look { face: Face::Black, fill: Fill::Solid(rgb(206, 17, 38)), plate: Some(INK), caps: true, mood: Some(Mood::Rise), ..PLAIN }),
    (&["yeezy", "yeezys"], Look { face: Face::Mono, fill: Fill::Solid(rgb(196, 177, 148)), spacing: 0.3, caps: true, ..PLAIN }),
    (&["chrome"], Look { face: Face::Gothic, fill: Fill::Chrome(SILVER), symbol: Some('\u{2720}'), ..PLAIN }),
    (&["versace"], Look { face: Face::Serif, fill: Fill::Fade(rgb(240, 210, 120), rgb(180, 130, 40)), spacing: 0.2, caps: true, ..PLAIN }),
    (&["dior"], Look { face: Face::Serif, spacing: 0.25, caps: true, ..PLAIN }),
    (&["chanel"], Look { spacing: 0.35, caps: true, plate: Some(INK), ..PLAIN }),
    (&["fendi"], Look { fill: Fill::Solid(rgb(255, 206, 0)), caps: true, ..PLAIN }),
    (&["hermes"], Look { face: Face::Serif, plate: Some(rgb(243, 112, 33)), caps: true, ..PLAIN }),
    (&["burberry"], Look { face: Face::Serif, fill: Fill::Solid(rgb(194, 160, 110)), stripes: &[INK, WHITE, rgb(170, 30, 40)], caps: true, ..PLAIN }),
    (&["moncler"], Look { stripes: &[rgb(0, 85, 164), WHITE, rgb(239, 65, 53)], caps: true, ..PLAIN }),
    (&["stoneisland"], Look { face: Face::Black, fill: Fill::Solid(rgb(232, 196, 50)), caps: true, ..PLAIN }),
    (&["rolex", "roli"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), plate: Some(rgb(0, 96, 57)), caps: true, symbol: Some('\u{265B}'), ..PLAIN }),
    (&["cartier"], Look { face: Face::Serif, plate: Some(rgb(167, 25, 48)), ..PLAIN }),
    (&["patek", "audemars", "piguet"], Look { face: Face::Serif, fill: Fill::Chrome(SILVER), spacing: 0.1, caps: true, ..PLAIN }),
    (&["ferrari"], Look { fill: Fill::Solid(rgb(220, 0, 0)), plate: Some(rgb(255, 226, 0)), caps: true, mood: Some(Mood::Fast), ..PLAIN }),
    (&["lambo", "lamborghini"], Look { face: Face::Black, fill: Fill::Solid(rgb(210, 170, 60)), slant: 0.12, caps: true, mood: Some(Mood::Fast), ..PLAIN }),
    (&["porsche"], Look { spacing: 0.4, caps: true, mood: Some(Mood::Fast), ..PLAIN }),
    (&["benz", "mercedes", "maybach", "bentley", "bentayga"], Look { face: Face::Serif, fill: Fill::Chrome(SILVER), caps: true, ..PLAIN }),
    (&["givenchy", "ysl", "laurent"], Look { face: Face::Serif, fill: Fill::Solid(WHITE), spacing: 0.18, caps: true, ..PLAIN }),
    (&["vlone"], Look { fill: Fill::Initial(rgb(255, 107, 0), WHITE), plate: Some(INK), caps: true, ..PLAIN }),
    (&["carhartt"], Look { fill: Fill::Solid(rgb(232, 160, 30)), caps: true, ..PLAIN }),
    (&["lacoste"], Look { fill: Fill::Solid(rgb(0, 130, 80)), caps: true, ..PLAIN }),
    (&["tommy", "hilfiger"], Look { stripes: &[rgb(0, 40, 110), WHITE, rgb(200, 16, 46)], caps: true, ..PLAIN }),

    // People, each their own way.
    (&["rihanna", "riri"], Look { face: Face::Serif, fill: Fill::Fade(rgb(200, 20, 60), rgb(255, 130, 175)), symbol: Some('\u{2602}'), ..PLAIN }),
    (&["asap", "a$ap", "flacko", "rocky"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(155, 89, 182), rgb(241, 196, 15)]), slant: -0.12, caps: true, ..PLAIN }),
    (&["drake", "drizzy"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), spacing: 0.2, caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["kanye", "ye", "yeezus"], Look { plate: Some(INK), spacing: 0.3, caps: true, symbol: Some('\u{271D}'), ..PLAIN }),
    (&["travis", "cactus"], Look { face: Face::Gothic, fill: Fill::Fade(rgb(180, 110, 40), rgb(255, 180, 70)), mood: Some(Mood::Fire), ..PLAIN }),
    (&["kendrick", "kdot", "k.dot"], Look { face: Face::Black, plate: Some(rgb(200, 30, 30)), caps: true, mood: Some(Mood::Stop), ..PLAIN }),
    (&["carti", "playboi"], Look { face: Face::Gothic, fill: Fill::Solid(rgb(220, 20, 40)), symbol: Some('\u{2020}'), mood: Some(Mood::Shake), ..PLAIN }),
    (&["uzi"], Look { fill: Fill::Letters(&[rgb(255, 92, 205), rgb(160, 90, 255)]), symbol: Some('\u{2605}'), mood: Some(Mood::Spin), ..PLAIN }),
    (&["pluto", "hendrix"], Look { fill: Fill::Fade(rgb(80, 140, 255), rgb(170, 90, 255)), symbol: Some('\u{2647}'), mood: Some(Mood::Rise), ..PLAIN }),
    (&["tupac", "2pac", "pac", "makaveli"], Look { face: Face::Gothic, plate: Some(INK), mood: Some(Mood::Dark), ..PLAIN }),
    (&["biggie", "notorious"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), symbol: Some('\u{265B}'), ..PLAIN }),
    (&["eminem", "shady"], Look { face: Face::Mono, caps: true, mood: Some(Mood::Shake), ..PLAIN }),
    (&["jay-z", "jayz", "hov", "hova"], Look { face: Face::Serif, fill: Fill::Chrome(SILVER), symbol: Some('\u{2666}'), caps: true, ..PLAIN }),
    (&["beyonce", "beyoncé"], Look { fill: Fill::Fade(rgb(255, 214, 102), rgb(214, 150, 40)), symbol: Some('\u{2726}'), mood: Some(Mood::Flash), ..PLAIN }),
    (&["taylor"], Look { face: Face::Serif, fill: Fill::Fade(rgb(255, 170, 210), rgb(150, 190, 255)), symbol: Some('\u{2727}'), ..PLAIN }),
    (&["billie"], Look { face: Face::Black, fill: Fill::Solid(rgb(170, 255, 0)), ..PLAIN }),
    (&["weeknd", "abel"], Look { fill: Fill::Solid(rgb(235, 20, 20)), mood: Some(Mood::Flash), ..PLAIN }),
    (&["wrld"], Look { face: Face::Mono, fill: Fill::Letters(&[rgb(255, 0, 80), rgb(255, 200, 0), rgb(0, 200, 255), rgb(150, 80, 255)]), caps: true, mood: Some(Mood::Wave), ..PLAIN }),
    (&["xxxtentacion"], Look { fill: Fill::Fade(rgb(60, 140, 255), WHITE), mood: Some(Mood::Dark), ..PLAIN }),
    (&["weezy", "wayne"], Look { face: Face::Mono, fill: Fill::Solid(rgb(255, 215, 0)), symbol: Some('\u{26A1}'), ..PLAIN }),
    (&["jackson", "mj"], Look { face: Face::Serif, symbol: Some('\u{2605}'), mood: Some(Mood::Spin), ..PLAIN }),
    (&["messi"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(117, 170, 219), WHITE]), caps: true, mood: Some(Mood::Fast), ..PLAIN }),
    (&["ronaldo", "cr7", "siuu", "siu"], Look { face: Face::Black, fill: Fill::Solid(GOLD), plate: Some(rgb(200, 16, 46)), caps: true, mood: Some(Mood::Rise), ..PLAIN }),
    (&["kobe", "mamba"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(85, 37, 130), rgb(253, 185, 39)]), caps: true, ..PLAIN }),
    (&["tyson"], Look { face: Face::Black, fill: Fill::Solid(rgb(220, 30, 30)), caps: true, mood: Some(Mood::Shake), ..PLAIN }),
    // Czech and Slovak scene.
    (&["yzomandias", "yzo", "yza", "yzu", "yzovi", "yzem", "jakub*", "cs:logic"], Look { face: Face::Gothic, fill: Fill::Fade(rgb(255, 222, 130), rgb(196, 140, 40)), symbol: Some('\u{2625}'), ..PLAIN }),
    (&["calin"], Look { face: Face::Black, fill: Fill::Fade(rgb(255, 120, 200), rgb(150, 90, 255)), symbol: Some('\u{2605}'), mood: Some(Mood::Flash), ..PLAIN }),
    (&["sheen"], Look { face: Face::Serif, fill: Fill::Chrome(rgb(200, 210, 230)), spacing: 0.1, ..PLAIN }),
    (&["tendo"], Look { face: Face::Mono, fill: Fill::Letters(&[rgb(120, 255, 90), WHITE]), caps: true, mood: Some(Mood::Spin), ..PLAIN }),
    (&["hasan"], Look { face: Face::Black, fill: Fill::Solid(rgb(230, 40, 40)), caps: true, ..PLAIN }),
    (&["separ"], Look { face: Face::Gothic, plate: Some(INK), ..PLAIN }),
    (&["marpo"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(110, 122, 70), rgb(160, 140, 90)]), caps: true, mood: Some(Mood::Shake), ..PLAIN }),
    (&["cristovao"], Look { fill: Fill::Fade(rgb(255, 210, 0), rgb(0, 170, 80)), mood: Some(Mood::Wave), ..PLAIN }),
    (&["barracuda", "sergei"], Look { face: Face::Gothic, fill: Fill::Solid(rgb(210, 30, 30)), mood: Some(Mood::Shake), ..PLAIN }),
    (&["ektor"], Look { face: Face::Serif, plate: Some(INK), caps: true, ..PLAIN }),
    (&["buka"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(0, 230, 255), rgb(255, 0, 170), WHITE]), caps: true, mood: Some(Mood::Shake), ..PLAIN }),
    (&["posse"], Look { face: Face::Gothic, fill: Fill::Chrome(SILVER), ..PLAIN }),
    (&["ptk"], Look { face: Face::Mono, fill: Fill::Solid(rgb(235, 40, 40)), caps: true, spacing: 0.2, ..PLAIN }),
    (&["saul"], Look { face: Face::Serif, fill: Fill::Fade(WHITE, rgb(120, 190, 255)), ..PLAIN }),
    (&["bulhar", "cashanova", "ca$hanova"], Look { face: Face::Black, fill: Fill::Solid(GOLD), caps: true, symbol: Some('$'), mood: Some(Mood::Money), ..PLAIN }),
    (&["shimmi"], Look { fill: Fill::Letters(&[rgb(255, 120, 200), rgb(90, 220, 255)]), mood: Some(Mood::Wave), ..PLAIN }),
    (&["58g"], Look { face: Face::Mono, fill: Fill::Solid(rgb(90, 230, 120)), caps: true, ..PLAIN }),
    (&["stein"], Look { face: Face::Black, fill: Fill::Chrome(SILVER), caps: true, ..PLAIN }),
    (&["zoot"], Look { fill: Fill::Fade(rgb(255, 170, 60), rgb(255, 90, 40)), ..PLAIN }),
    (&["rico"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), ..PLAIN }),
    (&["prync"], Look { face: Face::Black, fill: Fill::Solid(GOLD), mood: Some(Mood::Money), ..PLAIN }),

    // Places, the ones the songs keep naming, in their flags' colors.
    (&["praha", "prahu", "praze", "prahou", "prague", "pražsk*"], Look { face: Face::Black, stripes: &[rgb(255, 205, 0), rgb(214, 30, 40)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["brno", "brna", "brně", "brnem"], Look { face: Face::Black, stripes: &[rgb(214, 30, 40), WHITE], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["ostrava", "ostravy", "ostravě", "ostravu"], Look { face: Face::Black, stripes: &[rgb(0, 80, 160), rgb(255, 205, 0)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["česko", "česku", "česka", "čechy", "čech", "czech", "republik*"], Look { face: Face::Black, stripes: &[WHITE, rgb(215, 20, 26), rgb(17, 69, 126)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["bratislava", "bratislavy", "bratislavě", "blava", "blavu", "slovensko", "slovensku", "slovenska"], Look { face: Face::Black, stripes: &[WHITE, rgb(11, 78, 162), rgb(238, 28, 37)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["vídeň", "vídni", "vídně", "wien", "vienna", "rakousko", "rakousku", "austria"], Look { face: Face::Black, stripes: &[rgb(200, 16, 46), WHITE, rgb(200, 16, 46)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["berlín", "berlin", "berlíně", "mnichov", "mnichově", "munich", "německo", "německu", "germany"], Look { face: Face::Black, stripes: &[INK, rgb(221, 0, 0), rgb(255, 206, 0)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["itálie", "itálii", "italy", "italia", "milán", "miláně", "milano", "milan", "řím", "římě", "rome", "roma", "benátk*", "venice"], Look { face: Face::Serif, stripes: &[rgb(0, 146, 70), WHITE, rgb(206, 43, 55)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["paříž", "paříži", "paris", "francie", "francii", "france"], Look { face: Face::Serif, stripes: &[rgb(0, 85, 164), WHITE, rgb(239, 65, 53)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["londýn", "londýně", "london"], Look { face: Face::Black, stripes: &[rgb(200, 16, 46), WHITE, rgb(1, 33, 105)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["španělsk*", "spain", "madrid", "barcelon*"], Look { face: Face::Black, stripes: &[rgb(170, 21, 27), rgb(241, 191, 0), rgb(170, 21, 27)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["ibiza", "ibize", "ibizu"], Look { fill: Fill::Fade(rgb(255, 120, 200), rgb(80, 200, 255)), caps: true, mood: Some(Mood::Bounce), ..PLAIN }),
    (&["dubaj", "dubaji", "dubai"], Look { face: Face::Serif, fill: Fill::Solid(GOLD), stripes: &[rgb(0, 115, 47), WHITE, INK], caps: true, mood: Some(Mood::Money), ..PLAIN }),
    (&["monako", "monaku", "monaco", "monte"], Look { face: Face::Serif, stripes: &[rgb(206, 17, 38), WHITE], caps: true, mood: Some(Mood::Money), ..PLAIN }),
    (&["amsterdam*"], Look { face: Face::Black, stripes: &[rgb(174, 28, 40), WHITE, rgb(33, 70, 139)], caps: true, symbol: Some('\u{2618}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["miami"], Look { face: Face::Black, fill: Fill::Fade(rgb(255, 100, 190), rgb(60, 220, 255)), caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["vegas"], Look { face: Face::Serif, fill: Fill::Chrome(GOLD), caps: true, mood: Some(Mood::Flash), ..PLAIN }),
    (&["york", "nyc", "brooklyn", "bronx"], Look { face: Face::Black, stripes: &[rgb(0, 45, 114), rgb(255, 103, 31)], caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["atlanta", "atl"], Look { face: Face::Black, fill: Fill::Solid(rgb(225, 58, 62)), caps: true, mood: Some(Mood::Grow), ..PLAIN }),
    (&["alpy", "alp", "alpách", "alpama", "alps", "hory", "horách", "hor"], Look { face: Face::Black, fill: Fill::Fade(WHITE, rgb(150, 200, 255)), symbol: Some('\u{25B2}'), caps: true, mood: Some(Mood::Rise), ..PLAIN }),
    (&["pláž", "pláži", "pláže", "plážích", "beach", "beaches"], Look { fill: Fill::Solid(rgb(240, 214, 150)), symbol: Some('\u{2600}'), mood: Some(Mood::Wave), ..PLAIN }),

    // Words that bring a symbol along.
    (&["extáze", "extází", "extázi", "ecstasy", "xtc", "mdma", "molly", "emko", "emka", "emku", "emkem", "éčko", "éčka", "éčkem", "pill", "pills", "pilulk*", "mdmá"], Look { fill: Fill::Letters(&[rgb(255, 230, 60), rgb(255, 120, 200)]), symbol: Some('\u{263B}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["koks", "koksu", "koksem", "koko", "coke", "cocaine", "kokain*", "perník", "perníku", "perníkem", "pervitin*", "snow", "crack", "pyrex", "meth", "cs:meta", "cs:metu", "cs:metou", "méťa"], Look { fill: Fill::Solid(rgb(235, 245, 255)), symbol: Some('\u{2744}'), mood: Some(Mood::Cold), ..PLAIN }),
    (&["tráva", "trávu", "trávy", "trávou", "gandža", "gandžu", "gandži", "ganja", "weed", "kush", "joint", "jointy", "jointu", "jointa", "špek", "špeky", "špeka", "brko", "brka", "brkem", "skéro", "skéra", "skérem", "marihuan*", "marjána", "hulím", "hulíme", "hulit", "hulíš", "zhulen*", "zahulen*", "sativa", "indica"], Look { fill: Fill::Solid(rgb(110, 220, 90)), symbol: Some('\u{2618}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["vajglu", "cigarety", "cigareta", "cigaretu", "cigaret", "cigára", "cigáro", "cígo", "cíga", "cigarette*", "ciggy", "ciggies"], Look { fill: Fill::Fade(WHITE, rgb(255, 120, 40)), mood: Some(Mood::Fire), ..PLAIN }),
    (&["alkohol*", "alcohol"], Look { fill: Fill::Fade(rgb(255, 200, 80), rgb(200, 120, 40)), mood: Some(Mood::Wave), ..PLAIN }),
    (&["úsměv", "úsměvem", "úsměvu", "usměj*", "usmív*", "smile", "smiling", "šťastn*", "happy", "radost*"], Look { fill: Fill::Solid(rgb(255, 220, 60)), symbol: Some('\u{263A}'), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["zdechnu", "chcípnu", "umřu", "umírám", "zemřu", "dead", "death", "smrt", "smrti", "smrtí"], Look { symbol: Some('\u{2620}'), mood: Some(Mood::Dark), ..PLAIN }),
    (&["krev", "krve", "krví", "krvi", "krvavý", "krvavej", "blood", "bloody", "bleed", "bleeding"], Look { fill: Fill::Fade(rgb(220, 20, 30), rgb(120, 0, 10)), mood: Some(Mood::Fall), ..PLAIN }),
    (&["peklo", "pekla", "pekle", "peklem", "pekeln*", "hell"], Look { face: Face::Gothic, fill: Fill::Fade(rgb(255, 140, 30), rgb(200, 10, 10)), mood: Some(Mood::Fire), ..PLAIN }),
    (&["hříchy", "hřích", "hříchů", "sin", "sins"], Look { face: Face::Gothic, fill: Fill::Solid(rgb(200, 20, 30)), mood: Some(Mood::Dark), ..PLAIN }),
    (&["svatej", "svatý", "svatá", "svaté", "svatou", "holy", "blessed", "požehnan*"], Look { face: Face::Serif, fill: Fill::Chrome(rgb(255, 230, 160)), symbol: Some('\u{2629}'), mood: Some(Mood::Rise), ..PLAIN }),
    (&["pomoc", "pomoz", "pomož", "pomožte", "pomohl*", "pomoct", "pomůžeš", "help"], Look { fill: Fill::Letters(&[rgb(255, 60, 60), WHITE]), caps: true, mood: Some(Mood::Flash), ..PLAIN }),
    (&["zprávy", "zpráva", "zprávu", "zpráv", "zprávou", "message", "messages", "texts", "dms", "dm", "sms", "esemesk*"], Look { symbol: Some('\u{2709}'), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["dům", "domě", "domem", "house", "barák", "baráku", "barákem"], Look { symbol: Some('\u{2302}'), mood: Some(Mood::Love), ..PLAIN }),
    (&["wanted"], Look { face: Face::Serif, fill: Fill::Solid(rgb(70, 40, 20)), plate: Some(rgb(226, 200, 150)), spacing: 0.12, caps: true, mood: Some(Mood::Stop), ..PLAIN }),
    (&["tattoo", "tattoos", "kérk*", "tetování", "tetovan*"], Look { face: Face::Gothic, fill: Fill::Fade(rgb(130, 180, 255), rgb(60, 90, 160)), ..PLAIN }),
    (&["skip", "skipnem", "skipneme", "skipni", "skipnu", "skipuju", "skipujem", "skipped"], Look { face: Face::Mono, caps: true, symbol: Some('\u{23ED}'), mood: Some(Mood::Away), ..PLAIN }),
    (&["rozední", "rozednívá", "rozednění", "úsvit", "úsvitu", "sunrise", "svítání"], Look { fill: Fill::Fade(rgb(255, 200, 80), rgb(255, 110, 70)), symbol: Some('\u{2600}'), mood: Some(Mood::Rise), ..PLAIN }),
    (&["star", "stars", "superstar", "hvězd*"], Look { fill: Fill::Solid(rgb(255, 225, 120)), symbol: Some('\u{2605}'), mood: Some(Mood::Rise), ..PLAIN }),
    (&["ketamin*", "ketáč", "keta", "kéťo", "ket"], Look { fill: Fill::Fade(rgb(200, 150, 255), rgb(120, 90, 220)), mood: Some(Mood::Slow), ..PLAIN }),
    (&["mithril*"], Look { face: Face::Serif, fill: Fill::Chrome(rgb(190, 215, 255)), caps: true, mood: Some(Mood::Flash), ..PLAIN }),
    (&["serotonin*", "seratonin*", "dopamin*", "endorfin*", "adrenalin*"], Look { face: Face::Mono, fill: Fill::Letters(&[rgb(90, 230, 255), rgb(255, 110, 210)]), symbol: Some('\u{269B}'), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["heroin*", "herák", "heráč*", "fentanyl*", "fenťák"], Look { fill: Fill::Fade(rgb(200, 200, 210), rgb(110, 110, 125)), symbol: Some('\u{2620}'), mood: Some(Mood::Fall), ..PLAIN }),
    (&["xan", "xans", "xanax*", "xanny", "benzo*", "prášk*"], Look { fill: Fill::Solid(rgb(185, 205, 255)), mood: Some(Mood::Slow), ..PLAIN }),
    (&["fízl*", "fízlové", "cops", "cop", "police", "polic*", "policajt*", "benga", "bengy", "feds"], Look { face: Face::Black, fill: Fill::Letters(&[rgb(40, 110, 255), rgb(255, 40, 50)]), caps: true, mood: Some(Mood::Flash), ..PLAIN }),
    (&["vajgl*", "čvaňhák*"], Look { fill: Fill::Fade(WHITE, rgb(255, 120, 40)), mood: Some(Mood::Fire), ..PLAIN }),
    (&["káva", "kávu", "kávy", "kafe", "kafem", "kafíčko", "coffee", "espresso"], Look { fill: Fill::Solid(rgb(196, 146, 96)), symbol: Some('\u{2615}'), mood: Some(Mood::Shake), ..PLAIN }),
    (&["nahrávám", "nahrávat", "nahráv*", "studio", "studiu", "studia", "recording", "rec", "mic", "mikrák*", "mikrofon*"], Look { face: Face::Mono, fill: Fill::Solid(rgb(255, 70, 70)), caps: true, symbol: Some('\u{25CF}'), mood: Some(Mood::Flash), ..PLAIN }),
    (&["jídlo", "jídla", "jídlem", "food", "žrádlo", "žrádla", "žrát", "žeru"], Look { fill: Fill::Solid(rgb(255, 170, 70)), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["les", "lesa", "lese", "lesem", "lesy", "lesích", "forest", "woods"], Look { fill: Fill::Solid(rgb(90, 190, 100)), symbol: Some('\u{2663}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["práce", "práci", "prací", "makám", "makat", "makáme", "maká", "work", "working", "job", "šichta", "šichtu"], Look { face: Face::Mono, fill: Fill::Solid(rgb(255, 185, 60)), caps: true, mood: Some(Mood::Stop), ..PLAIN }),
    (&["múza", "múzu", "múzo", "múzou", "muse"], Look { face: Face::Serif, fill: Fill::Fade(rgb(255, 200, 230), rgb(255, 220, 150)), symbol: Some('\u{2727}'), mood: Some(Mood::Love), ..PLAIN }),
    (&["batman*"], Look { face: Face::Black, fill: Fill::Solid(rgb(255, 220, 0)), plate: Some(INK), caps: true, mood: Some(Mood::Dark), ..PLAIN }),
    (&["jednoro*", "unicorn*"], Look { fill: Fill::Letters(&[rgb(255, 90, 90), rgb(255, 200, 60), rgb(90, 220, 120), rgb(80, 170, 255), rgb(190, 110, 255)]), symbol: Some('\u{2726}'), mood: Some(Mood::Rise), ..PLAIN }),
    (&["cs:had", "hada", "hadi", "hadem", "hadů", "hady", "zmije", "zmiji", "zmijí", "snake", "snakes", "viper*"], Look { fill: Fill::Fade(rgb(120, 220, 90), rgb(40, 130, 60)), mood: Some(Mood::Wave), ..PLAIN }),
    (&["wake", "waking", "vstávej", "vstávat", "vstávám", "probuď*", "probouz*", "budík*"], Look { fill: Fill::Solid(rgb(255, 240, 150)), symbol: Some('\u{263C}'), mood: Some(Mood::Flash), ..PLAIN }),
    (&["sleep", "sleeping", "spát", "spím", "spíš", "spíme", "usnu", "usnul*", "usnout", "spánek", "spánku"], Look { fill: Fill::Fade(rgb(170, 180, 255), rgb(110, 120, 200)), symbol: Some('\u{263E}'), mood: Some(Mood::Slow), ..PLAIN }),
    (&["gratuluju", "gratuluji", "gratulace", "gratuluj*", "congrats", "congratulations"], Look { fill: Fill::Letters(&[rgb(255, 90, 140), rgb(255, 210, 60), rgb(80, 200, 255), rgb(130, 230, 110)]), symbol: Some('\u{2727}'), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["sold", "soldout", "vyprodan*", "vyprodáno"], Look { face: Face::Black, plate: Some(rgb(214, 30, 40)), caps: true, mood: Some(Mood::Stop), ..PLAIN }),
    (&["čauš", "čaus", "čau", "čaues", "čus"], Look { face: Face::Gothic, fill: Fill::Fade(rgb(255, 222, 130), rgb(196, 140, 40)), mood: Some(Mood::Bounce), ..PLAIN }),
    (&["ankh", "ankhy"], Look { fill: Fill::Solid(GOLD), caps: true, symbol: Some('\u{2625}'), ..PLAIN }),
    (&["cross", "kříž", "=kříže", "křížem", "křížek", "jesus", "ježíš*", "kristus", "krista", "god", "lord", "bůh", "boha", "bohu", "bože", "bohem"], Look { symbol: Some('\u{271D}'), ..PLAIN }),
    (&["time", "times", "čas", "času", "časem", "clock", "hodin", "hodiny", "hodinu", "hodinách", "hodinama"], Look { symbol: Some('\u{231B}'), mood: Some(Mood::Slow), ..PLAIN }),
    (&["crown", "koruna", "korunu", "korunou", "king", "trůn", "trůnu", "trůně", "throne", "král", "krále", "králi", "králem", "králové", "královna", "královno"], Look { fill: Fill::Solid(GOLD), symbol: Some('\u{265B}'), mood: Some(Mood::Grow), ..PLAIN }),
    (&["skull", "lebka", "lebku", "poison", "toxic", "toxick*"], Look { symbol: Some('\u{2620}'), mood: Some(Mood::Dark), ..PLAIN }),
    (&["peace"], Look { symbol: Some('\u{262E}'), ..PLAIN }),
    (&["karma", "yin", "yang"], Look { symbol: Some('\u{262F}'), mood: Some(Mood::Spin), ..PLAIN }),
    (&["infinity", "nekonečn*"], Look { symbol: Some('\u{221E}'), mood: Some(Mood::Grow), ..PLAIN }),
    (&["lightning", "thunder", "blesk*"], Look { fill: Fill::Solid(rgb(255, 230, 80)), symbol: Some('\u{26A1}'), mood: Some(Mood::Flash), ..PLAIN }),
    (&["umbrella", "deštník*"], Look { symbol: Some('\u{2602}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["snowflake", "vločk*"], Look { fill: Fill::Solid(rgb(175, 228, 255)), symbol: Some('\u{2744}'), mood: Some(Mood::Cold), ..PLAIN }),
    (&["rose", "roses", "růže", "růži", "flower*", "květin*"], Look { fill: Fill::Solid(rgb(235, 60, 90)), symbol: Some('\u{273F}'), ..PLAIN }),
    (&["plane", "jet", "letadl*", "cs:privát*"], Look { symbol: Some('\u{2708}'), mood: Some(Mood::Fast), ..PLAIN }),
    (&["phone", "telefon*", "volám", "voláš", "zavol*", "volej"], Look { symbol: Some('\u{260E}'), mood: Some(Mood::Shake), ..PLAIN }),
    (&["danger", "nebezpeč*"], Look { fill: Fill::Solid(rgb(255, 210, 0)), symbol: Some('\u{26A0}'), ..PLAIN }),
    (&["music", "hudb*", "melodi*"], Look { symbol: Some('\u{266B}'), mood: Some(Mood::Wave), ..PLAIN }),
    (&["666", "satan*"], Look { face: Face::Gothic, fill: Fill::Solid(rgb(200, 0, 0)), symbol: Some('\u{26E7}'), mood: Some(Mood::Dark), ..PLAIN }),
    (&["anchor", "kotv*"], Look { symbol: Some('\u{2693}'), mood: Some(Mood::Fall), ..PLAIN }),
];

static PREPARED: LazyLock<Vec<(Vec<Entry>, &'static Look)>> =
    LazyLock::new(|| LOOKS.iter().map(|(names, look)| (prepare(names), look)).collect());

/// The look for a word, if it is dressed.
pub(super) fn look(token: &Token) -> Option<&'static Look> {
    PREPARED.iter().find(|(names, _)| listed(token, names)).map(|(_, look)| *look)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_find_their_look() {
        let look = |w: &str| look(&Token::new(w, true));
        let face = |w: &str| look(w).map(|l| l.face);
        assert_eq!(face("GUCCI"), Some(Face::Serif));
        assert_eq!(face("Rihanna's"), Some(Face::Serif));
        assert_eq!(face("A$AP"), Some(Face::Black));
        assert!(look("ankh").is_some_and(|l| l.symbol == Some('\u{2625}')));
        assert!(look("Yzomandias").is_some_and(|l| l.symbol == Some('\u{2625}')));
        assert!(look("korunu").is_some_and(|l| l.symbol.is_some()));
        assert!(look("table").is_none());
        assert!(look("čauš").is_some_and(|l| l.face == Face::Gothic));
        assert!(look("Logic").is_some_and(|l| l.symbol == Some('\u{2625}')));
        assert!(super::look(&Token::new("logic", false)).is_none());
        // Everyday words that only fold into a listed one stay plain.
        for word in ["krize", "králík"] {
            assert!(look(word).is_none(), "{word}");
        }
        for word in ["juice", "slim", "stone", "saint", "private"] {
            assert!(super::look(&Token::new(word, false)).is_none(), "{word}");
        }
    }
}
