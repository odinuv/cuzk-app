use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DruhCislovani {
    Stavebni,
    Pozemkova,
}

impl DruhCislovani {
    pub fn api_code(&self) -> u8 {
        match self {
            Self::Stavebni => 1,
            Self::Pozemkova => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypStavby {
    Cp,
    Ce,
}

impl TypStavby {
    pub fn api_code(&self) -> u8 {
        match self {
            Self::Cp => 1,
            Self::Ce => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParcelaRef {
    pub druh: DruhCislovani,
    pub kmenove: u32,
    pub poddeleni: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemIdent {
    Parcela {
        ku: u32,
        parcela: ParcelaRef,
    },
    Stavba {
        ku: u32,
        typ: TypStavby,
        cislo_domovni: u32,
        na_parcele: ParcelaRef,
    },
    Jednotka {
        ku: u32,
        typ: TypStavby,
        cislo_domovni: u32,
        cislo_jednotky: u32,
        kod_casti_obce: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub popis: String,
    pub enabled: bool,
    pub ident: ItemIdent,
}

/// Stable fingerprint of the identification only (not popis/enabled/id).
pub fn item_key(ident: &ItemIdent) -> String {
    serde_json::to_string(ident).expect("ItemIdent serialization cannot fail")
}

fn parcela_text(p: &ParcelaRef) -> String {
    let prefix = match p.druh {
        DruhCislovani::Stavebni => "st.",
        DruhCislovani::Pozemkova => "p.č.",
    };
    match p.poddeleni {
        Some(d) => format!("{prefix} {}/{d}", p.kmenove),
        None => format!("{prefix} {}", p.kmenove),
    }
}

fn typ_text(t: TypStavby) -> &'static str {
    match t {
        TypStavby::Cp => "č.p.",
        TypStavby::Ce => "č.e.",
    }
}

pub fn describe(ident: &ItemIdent) -> String {
    match ident {
        ItemIdent::Parcela { ku, parcela } => {
            format!("parcela {}, k.ú. {ku}", parcela_text(parcela))
        }
        ItemIdent::Stavba { ku, typ, cislo_domovni, na_parcele } => format!(
            "stavba {} {cislo_domovni} na {}, k.ú. {ku}",
            typ_text(*typ),
            parcela_text(na_parcele)
        ),
        ItemIdent::Jednotka { ku, cislo_domovni, cislo_jednotky, .. } => {
            format!("jednotka {cislo_domovni}/{cislo_jednotky}, k.ú. {ku}")
        }
    }
}

fn validate_parcela(p: &ParcelaRef) -> Result<(), String> {
    if p.kmenove == 0 {
        return Err("Kmenové číslo parcely musí být větší než 0.".into());
    }
    Ok(())
}

pub fn validate(ident: &ItemIdent) -> Result<(), String> {
    let check_ku = |ku: u32| {
        if ku == 0 {
            Err("Kód katastrálního území musí být větší než 0.".to_string())
        } else {
            Ok(())
        }
    };
    match ident {
        ItemIdent::Parcela { ku, parcela } => {
            check_ku(*ku)?;
            validate_parcela(parcela)
        }
        ItemIdent::Stavba { ku, cislo_domovni, na_parcele, .. } => {
            check_ku(*ku)?;
            if *cislo_domovni == 0 {
                return Err("Číslo domovní musí být větší než 0.".into());
            }
            validate_parcela(na_parcele)
        }
        ItemIdent::Jednotka { ku, cislo_domovni, cislo_jednotky, .. } => {
            check_ku(*ku)?;
            if *cislo_domovni == 0 {
                return Err("Číslo domovní musí být větší než 0.".into());
            }
            if *cislo_jednotky == 0 {
                return Err("Číslo jednotky musí být větší než 0.".into());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parcela(druh: DruhCislovani, k: u32, p: Option<u32>) -> ParcelaRef {
        ParcelaRef { druh, kmenove: k, poddeleni: p }
    }

    fn pid(p: Option<u32>) -> ItemIdent {
        ItemIdent::Parcela { ku: 618853, parcela: parcela(DruhCislovani::Stavebni, 182, p) }
    }

    #[test]
    fn key_ignores_popis_and_enabled() {
        let a = Item { id: 1, popis: "a".into(), enabled: true, ident: pid(Some(2)) };
        let b = Item { id: 2, popis: "b".into(), enabled: false, ident: pid(Some(2)) };
        assert_eq!(item_key(&a.ident), item_key(&b.ident));
    }

    #[test]
    fn key_distinguishes_poddeleni_none_vs_zero() {
        assert_ne!(item_key(&pid(None)), item_key(&pid(Some(0))));
    }

    #[test]
    fn api_codes() {
        assert_eq!(DruhCislovani::Stavebni.api_code(), 1);
        assert_eq!(DruhCislovani::Pozemkova.api_code(), 2);
        assert_eq!(TypStavby::Cp.api_code(), 1);
        assert_eq!(TypStavby::Ce.api_code(), 2);
    }

    #[test]
    fn describe_examples() {
        assert_eq!(describe(&pid(Some(2))), "parcela st. 182/2, k.ú. 618853");
        let s = ItemIdent::Stavba {
            ku: 738395,
            typ: TypStavby::Ce,
            cislo_domovni: 3,
            na_parcele: parcela(DruhCislovani::Pozemkova, 110, Some(10)),
        };
        assert_eq!(describe(&s), "stavba č.e. 3 na p.č. 110/10, k.ú. 738395");
        let j = ItemIdent::Jednotka {
            ku: 610704,
            typ: TypStavby::Cp,
            cislo_domovni: 893,
            cislo_jednotky: 37,
            kod_casti_obce: None,
        };
        assert_eq!(describe(&j), "jednotka 893/37, k.ú. 610704");
        let p = ItemIdent::Parcela {
            ku: 738395,
            parcela: parcela(DruhCislovani::Pozemkova, 110, Some(10)),
        };
        assert_eq!(describe(&p), "parcela p.č. 110/10, k.ú. 738395");
    }

    #[test]
    fn validate_rejects_zero() {
        let bad = ItemIdent::Parcela {
            ku: 1,
            parcela: parcela(DruhCislovani::Stavebni, 0, None),
        };
        assert!(validate(&bad).is_err());
        let bad = ItemIdent::Stavba {
            ku: 1,
            typ: TypStavby::Cp,
            cislo_domovni: 0,
            na_parcele: parcela(DruhCislovani::Stavebni, 1, None),
        };
        assert!(validate(&bad).is_err());
        assert!(validate(&pid(Some(2))).is_ok());
    }
}
