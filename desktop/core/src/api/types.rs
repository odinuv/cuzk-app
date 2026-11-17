use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T> {
    pub data: T,
    #[serde(default)]
    pub aktualnost_dat_k: Option<String>,
    #[serde(default)]
    pub provedeno_volani: Option<u32>,
}

/// Flat (non-enveloped) body of `GET /AplikacniSluzby/StavUctu`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StavUctu {
    pub provedeno_volani: u32,
    pub limit_volani: u32,
    #[serde(default)]
    pub expirace_api_key: Option<String>,
}

/// A proceeding (plomba) as listed in `rizeniPlomby`. Only `id` is required: it is what
/// plomby are matched by, the rest is for display and may be missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RizeniDef {
    pub id: i64,
    #[serde(default)]
    pub typ_rizeni: Option<String>,
    #[serde(default)]
    pub poradove_cislo: Option<u32>,
    #[serde(default)]
    pub rok: Option<u32>,
    #[serde(default)]
    pub kod_pracoviste: Option<u32>,
}

impl RizeniDef {
    /// e.g. `V-1234/2026-701`; missing parts are `?`, and with none of them known
    /// `řízení č. <id>`.
    pub fn label(&self) -> String {
        fn part<T: ToString>(v: &Option<T>) -> String {
            v.as_ref().map_or_else(|| "?".to_string(), T::to_string)
        }
        if self.typ_rizeni.is_none()
            && self.poradove_cislo.is_none()
            && self.rok.is_none()
            && self.kod_pracoviste.is_none()
        {
            return format!("řízení č. {}", self.id);
        }
        format!(
            "{}-{}/{}-{}",
            part(&self.typ_rizeni),
            part(&self.poradove_cislo),
            part(&self.rok),
            part(&self.kod_pracoviste)
        )
    }
}
