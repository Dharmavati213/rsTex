//! The format table: tex-file.c's `kpathsea_init_format_return_varlist`, as data.

/// One row of `kpse_file_format_type`, tex-file.h.
pub const KPSE_GF_FORMAT: i32 = 0;
pub const KPSE_PK_FORMAT: i32 = 1;
pub const KPSE_ANY_GLYPH_FORMAT: i32 = 2;
pub const KPSE_TFM_FORMAT: i32 = 3;
pub const KPSE_AFM_FORMAT: i32 = 4;
pub const KPSE_BASE_FORMAT: i32 = 5;
pub const KPSE_BIB_FORMAT: i32 = 6;
pub const KPSE_BST_FORMAT: i32 = 7;
pub const KPSE_CNF_FORMAT: i32 = 8;
pub const KPSE_DB_FORMAT: i32 = 9;
pub const KPSE_FMT_FORMAT: i32 = 10;
pub const KPSE_FONTMAP_FORMAT: i32 = 11;
pub const KPSE_MEM_FORMAT: i32 = 12;
pub const KPSE_MF_FORMAT: i32 = 13;
pub const KPSE_MFPOOL_FORMAT: i32 = 14;
pub const KPSE_MFT_FORMAT: i32 = 15;
pub const KPSE_MP_FORMAT: i32 = 16;
pub const KPSE_MPPOOL_FORMAT: i32 = 17;
pub const KPSE_MPSUPPORT_FORMAT: i32 = 18;
pub const KPSE_OCP_FORMAT: i32 = 19;
pub const KPSE_OFM_FORMAT: i32 = 20;
pub const KPSE_OPL_FORMAT: i32 = 21;
pub const KPSE_OTP_FORMAT: i32 = 22;
pub const KPSE_OVF_FORMAT: i32 = 23;
pub const KPSE_OVP_FORMAT: i32 = 24;
pub const KPSE_PICT_FORMAT: i32 = 25;
pub const KPSE_TEX_FORMAT: i32 = 26;
pub const KPSE_TEXDOC_FORMAT: i32 = 27;
pub const KPSE_TEXPOOL_FORMAT: i32 = 28;
pub const KPSE_TEXSOURCE_FORMAT: i32 = 29;
pub const KPSE_TEX_PS_HEADER_FORMAT: i32 = 30;
pub const KPSE_TROFF_FONT_FORMAT: i32 = 31;
pub const KPSE_TYPE1_FORMAT: i32 = 32;
pub const KPSE_VF_FORMAT: i32 = 33;
pub const KPSE_DVIPS_CONFIG_FORMAT: i32 = 34;
pub const KPSE_IST_FORMAT: i32 = 35;
pub const KPSE_TRUETYPE_FORMAT: i32 = 36;
pub const KPSE_TYPE42_FORMAT: i32 = 37;
pub const KPSE_WEB2C_FORMAT: i32 = 38;
pub const KPSE_PROGRAM_TEXT_FORMAT: i32 = 39;
pub const KPSE_PROGRAM_BINARY_FORMAT: i32 = 40;
pub const KPSE_MISCFONTS_FORMAT: i32 = 41;
pub const KPSE_WEB_FORMAT: i32 = 42;
pub const KPSE_CWEB_FORMAT: i32 = 43;
pub const KPSE_ENC_FORMAT: i32 = 44;
pub const KPSE_CMAP_FORMAT: i32 = 45;
pub const KPSE_SFD_FORMAT: i32 = 46;
pub const KPSE_OPENTYPE_FORMAT: i32 = 47;
pub const KPSE_PDFTEX_CONFIG_FORMAT: i32 = 48;
pub const KPSE_LIG_FORMAT: i32 = 49;
pub const KPSE_TEXMFSCRIPTS_FORMAT: i32 = 50;
pub const KPSE_LUA_FORMAT: i32 = 51;
pub const KPSE_FEA_FORMAT: i32 = 52;
pub const KPSE_CID_FORMAT: i32 = 53;
pub const KPSE_MLBIB_FORMAT: i32 = 54;
pub const KPSE_MLBST_FORMAT: i32 = 55;
pub const KPSE_CLUA_FORMAT: i32 = 56;
pub const KPSE_RIS_FORMAT: i32 = 57;
pub const KPSE_BLTXML_FORMAT: i32 = 58;
/// `kpse_last_format`: one past the last index.
pub const KPSE_LAST_FORMAT: i32 = 59;

/// The compile-time paths of paths.h.  They come out of `cnf-to-paths.awk`, and the oracle's table
/// shows (`TEXMFCNF=/nonexistent kpsewhich --help-formats`) that all but these are `/nonesuch`: a
/// system without a `texmf.cnf` has no search path, not even `.`.
pub const DEFAULT_NONESUCH: &[u8] = b"/nonesuch";
/// `DEFAULT_TEXMFCNF`.
pub const DEFAULT_TEXMFCNF: &[u8] = b"{$SELFAUTOLOC,$SELFAUTOLOC/share/texmf-local/web2c,$SELFAUTOLOC/share/texmf-dist/web2c,$SELFAUTOLOC/share/texmf/web2c,$SELFAUTOLOC/texmf-local/web2c,$SELFAUTOLOC/texmf-dist/web2c,$SELFAUTOLOC/texmf/web2c,$SELFAUTODIR,$SELFAUTODIR/share/texmf-local/web2c,$SELFAUTODIR/share/texmf-dist/web2c,$SELFAUTODIR/share/texmf/web2c,$SELFAUTODIR/texmf-local/web2c,$SELFAUTODIR/texmf-dist/web2c,$SELFAUTODIR/texmf/web2c,$SELFAUTOGRANDPARENT/texmf-local/web2c,$SELFAUTOPARENT,$SELFAUTOPARENT/share/texmf-local/web2c,$SELFAUTOPARENT/share/texmf-dist/web2c,$SELFAUTOPARENT/share/texmf/web2c,$SELFAUTOPARENT/texmf-local/web2c,$SELFAUTOPARENT/texmf-dist/web2c,$SELFAUTOPARENT/texmf/web2c}";
/// `DEFAULT_TRFONTS`.
pub const DEFAULT_TRFONTS: &[u8] = b"/usr{/local,}/share/groff/{current/font,site-font}/devps";
/// `DEFAULT_CLUAINPUTS`.
pub const DEFAULT_CLUAINPUTS: &[u8] = b"$TEXMFDOTDIR:$SELFAUTOLOC/lib/{$progname,$engine,}/lua//";

/// The compile-time default of a format, before anything else is consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultPath {
    Nonesuch,
    Cnf,
    TrFonts,
    CLua,
    /// `.:$TEXMF/<program name>//`.
    Program,
}

/// One format.  `envs` empty and `program_envs` true means `<PROGRAM NAME>INPUTS`.
#[derive(Clone, Copy, Debug)]
pub struct FormatDef {
    pub name: &'static str,
    pub default: DefaultPath,
    pub envs: &'static [&'static str],
    pub program_envs: bool,
    pub suffix: &'static [&'static str],
    pub alt_suffix: &'static [&'static str],
    pub suffix_search_only: bool,
    pub binmode: bool,
}

const fn f(
    name: &'static str,
    envs: &'static [&'static str],
    suffix: &'static [&'static str],
    alt_suffix: &'static [&'static str],
    suffix_search_only: bool,
    binmode: bool,
) -> FormatDef {
    FormatDef { name, default: DefaultPath::Nonesuch, envs, program_envs: false, suffix, alt_suffix, suffix_search_only, binmode }
}

const fn with_default(mut d: FormatDef, default: DefaultPath) -> FormatDef {
    d.default = default;
    d
}

const NONE: &[&str] = &[];

/// Indexed by format code.
pub static FORMATS: [FormatDef; KPSE_LAST_FORMAT as usize] = [
    f("gf", &["GFFONTS", "GLYPHFONTS", "TEXFONTS"], &["gf"], NONE, true, true),
    f("pk", &["PKFONTS", "TEXPKS", "GLYPHFONTS", "TEXFONTS"], &["pk"], NONE, true, true),
    f("bitmap font", &["GLYPHFONTS", "TEXFONTS"], NONE, NONE, true, true),
    f("tfm", &["TFMFONTS", "TEXFONTS"], &[".tfm"], NONE, true, true),
    f("afm", &["AFMFONTS", "TEXFONTS"], &[".afm"], NONE, false, false),
    f("base", &["MFBASES", "TEXMFINI"], &[".base"], NONE, false, true),
    f("bib", &["BIBINPUTS", "TEXBIB"], &[".bib"], NONE, true, false),
    f("bst", &["BSTINPUTS"], &[".bst"], NONE, false, false),
    with_default(f("cnf", &["TEXMFCNF"], &[".cnf"], NONE, false, false), DefaultPath::Cnf),
    f("ls-R", &["TEXMFDBS"], &["ls-R", "ls-r"], NONE, false, false),
    f("fmt", &["TEXFORMATS", "TEXMFINI"], &[".fmt"], NONE, false, true),
    f("map", &["TEXFONTMAPS", "TEXFONTS"], &[".map"], NONE, false, false),
    f("mem", &["MPMEMS", "TEXMFINI"], &[".mem"], NONE, false, true),
    f("mf", &["MFINPUTS"], &[".mf"], NONE, false, false),
    f("mfpool", &["MFPOOL", "TEXMFINI"], &[".pool"], NONE, false, false),
    f("mft", &["MFTINPUTS"], &[".mft"], NONE, false, false),
    f("mp", &["MPINPUTS"], &[".mp"], NONE, false, false),
    f("mppool", &["MPPOOL", "TEXMFINI"], &[".pool"], NONE, false, false),
    f("MetaPost support", &["MPSUPPORT"], NONE, NONE, false, false),
    f("ocp", &["OCPINPUTS"], &[".ocp"], NONE, true, true),
    f("ofm", &["OFMFONTS", "TEXFONTS"], &[".ofm", ".tfm"], NONE, true, true),
    f("opl", &["OPLFONTS", "TEXFONTS"], &[".opl"], &[".pl"], true, false),
    f("otp", &["OTPINPUTS"], &[".otp"], NONE, true, false),
    f("ovf", &["OVFFONTS", "TEXFONTS"], &[".ovf", ".vf"], NONE, true, true),
    f("ovp", &["OVPFONTS", "TEXFONTS"], &[".ovp"], &[".vpl"], true, false),
    f("graphic/figure", &["TEXPICTS", "TEXINPUTS"], NONE, &[".eps", ".epsi"], false, true),
    f("tex", &["TEXINPUTS"], &[".tex"], &[".sty", ".cls", ".fd", ".aux", ".bbl", ".def", ".clo", ".ldf"], false, false),
    f("TeX system documentation", &["TEXDOCS"], NONE, NONE, false, false),
    f("texpool", &["TEXPOOL", "TEXMFINI"], &[".pool"], NONE, false, false),
    f("TeX system sources", &["TEXSOURCES"], NONE, &[".dtx", ".ins"], false, false),
    f("PostScript header", &["TEXPSHEADERS", "PSHEADERS"], NONE, &[".pro"], false, true),
    with_default(f("Troff fonts", &["TRFONTS"], NONE, NONE, false, true), DefaultPath::TrFonts),
    f("type1 fonts", &["T1FONTS", "T1INPUTS", "TEXFONTS", "TEXPSHEADERS", "PSHEADERS"], &[".pfa", ".pfb"], NONE, false, true),
    f("vf", &["VFFONTS", "TEXFONTS"], &[".vf"], NONE, true, true),
    f("dvips config", &["TEXCONFIG"], NONE, NONE, false, false),
    f("ist", &["TEXINDEXSTYLE", "INDEXSTYLE"], &[".ist"], NONE, false, false),
    f("truetype fonts", &["TTFONTS", "TEXFONTS"], &[".ttf", ".ttc", ".TTF", ".TTC", ".dfont"], NONE, false, true),
    f("type42 fonts", &["T42FONTS", "TEXFONTS"], &[".t42", ".T42"], NONE, false, true),
    f("web2c files", &["WEB2C"], NONE, NONE, false, false),
    FormatDef {
        name: "other text files",
        default: DefaultPath::Program,
        envs: NONE,
        program_envs: true,
        suffix: NONE,
        alt_suffix: NONE,
        suffix_search_only: false,
        binmode: false,
    },
    FormatDef {
        name: "other binary files",
        default: DefaultPath::Program,
        envs: NONE,
        program_envs: true,
        suffix: NONE,
        alt_suffix: NONE,
        suffix_search_only: false,
        binmode: true,
    },
    f("misc fonts", &["MISCFONTS", "TEXFONTS"], NONE, NONE, false, true),
    f("web", &["WEBINPUTS"], &[".web"], &[".ch"], false, false),
    f("cweb", &["CWEBINPUTS"], &[".w", ".web"], &[".ch"], false, false),
    f("enc files", &["ENCFONTS", "TEXFONTS"], &[".enc"], NONE, true, false),
    f("cmap files", &["CMAPFONTS", "TEXFONTS"], NONE, NONE, false, false),
    f("subfont definition files", &["SFDFONTS", "TEXFONTS"], &[".sfd"], NONE, true, false),
    f("opentype fonts", &["OPENTYPEFONTS", "TEXFONTS"], &[".otf", ".OTF"], NONE, true, true),
    f("pdftex config", &["PDFTEXCONFIG"], NONE, NONE, false, false),
    f("lig files", &["LIGFONTS", "TEXFONTS"], &[".lig"], NONE, true, false),
    f("texmfscripts", &["TEXMFSCRIPTS"], NONE, NONE, false, false),
    f("lua", &["LUAINPUTS"], &[".lua", ".luatex", ".luc", ".luctex", ".texlua", ".texluc", ".tlu"], NONE, true, false),
    f("font feature files", &["FONTFEATURES"], &[".fea"], NONE, true, false),
    f("cid maps", &["FONTCIDMAPS"], &[".cid", ".cidmap"], NONE, true, false),
    f("mlbib", &["MLBIBINPUTS", "BIBINPUTS", "TEXBIB"], &[".mlbib", ".bib"], NONE, true, false),
    f("mlbst", &["MLBSTINPUTS", "BSTINPUTS"], &[".mlbst", ".bst"], NONE, true, false),
    with_default(f("clua", &["CLUAINPUTS"], &[".dll", ".so"], NONE, true, false), DefaultPath::CLua),
    f("ris", &["RISINPUTS"], &[".ris"], NONE, true, false),
    f("bltxml", &["BLTXMLINPUTS"], &[".bltxml"], NONE, true, false),
];

/// The table's row for a format code.
pub fn format_def(format: i32) -> Option<&'static FormatDef> {
    usize::try_from(format).ok().and_then(|i| FORMATS.get(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_index_the_table() {
        // The codes WEB and openclose.c use, and the row each of them must land on.
        for (code, name) in [
            (KPSE_GF_FORMAT, "gf"),
            (KPSE_TFM_FORMAT, "tfm"),
            (KPSE_CNF_FORMAT, "cnf"),
            (KPSE_DB_FORMAT, "ls-R"),
            (KPSE_FMT_FORMAT, "fmt"),
            (KPSE_FONTMAP_FORMAT, "map"),
            (KPSE_OFM_FORMAT, "ofm"),
            (KPSE_PICT_FORMAT, "graphic/figure"),
            (KPSE_TEX_FORMAT, "tex"),
            (KPSE_TYPE1_FORMAT, "type1 fonts"),
            (KPSE_VF_FORMAT, "vf"),
            (KPSE_TRUETYPE_FORMAT, "truetype fonts"),
            (KPSE_PROGRAM_TEXT_FORMAT, "other text files"),
            (KPSE_ENC_FORMAT, "enc files"),
            (KPSE_OPENTYPE_FORMAT, "opentype fonts"),
            (KPSE_PDFTEX_CONFIG_FORMAT, "pdftex config"),
            (KPSE_BLTXML_FORMAT, "bltxml"),
        ] {
            assert_eq!(format_def(code).unwrap().name, name);
        }
        assert!(format_def(-1).is_none());
        assert!(format_def(KPSE_LAST_FORMAT).is_none());
    }
}
