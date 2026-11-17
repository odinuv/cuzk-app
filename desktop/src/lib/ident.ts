import type { DruhCislovani, ItemIdent, ItemKind, ParcelaRef, TypStavby } from "./api";

export interface ParcelaForm {
  druh: DruhCislovani;
  kmenove: string;
  poddeleni: string;
}

export interface IdentForm {
  kind: ItemKind;
  ku: number | null;
  parcela: ParcelaForm; // parcela itself, or the parcel under a stavba
  typ: TypStavby;
  cisloDomovni: string;
  cisloJednotky: string;
  kodCastiObce: number | null;
}

export function emptyIdentForm(): IdentForm {
  return {
    kind: "parcela",
    ku: null,
    parcela: { druh: "Pozemkova", kmenove: "", poddeleni: "" },
    typ: "Cp",
    cisloDomovni: "",
    cisloJednotky: "",
    kodCastiObce: null,
  };
}

function parcelaToForm(p: ParcelaRef): ParcelaForm {
  return { druh: p.druh, kmenove: String(p.kmenove), poddeleni: p.poddeleni === null ? "" : String(p.poddeleni) };
}

export function identToForm(ident: ItemIdent): IdentForm {
  const f = emptyIdentForm();
  if ("Parcela" in ident) {
    return { ...f, kind: "parcela", ku: ident.Parcela.ku, parcela: parcelaToForm(ident.Parcela.parcela) };
  }
  if ("Stavba" in ident) {
    const s = ident.Stavba;
    return { ...f, kind: "stavba", ku: s.ku, typ: s.typ, cisloDomovni: String(s.cislo_domovni), parcela: parcelaToForm(s.na_parcele) };
  }
  const j = ident.Jednotka;
  return {
    ...f,
    kind: "jednotka",
    ku: j.ku,
    typ: j.typ,
    cisloDomovni: String(j.cislo_domovni),
    cisloJednotky: String(j.cislo_jednotky),
    kodCastiObce: j.kod_casti_obce,
  };
}

function posInt(s: string, label: string): number {
  const t = s.trim();
  if (!/^\d+$/.test(t) || Number(t) <= 0) throw new Error(`${label}: zadejte kladné celé číslo.`);
  return Number(t);
}

function parcelaFromForm(p: ParcelaForm): ParcelaRef {
  const pod = p.poddeleni.trim();
  return {
    druh: p.druh,
    kmenove: posInt(p.kmenove, "Kmenové číslo parcely"),
    poddeleni: pod === "" ? null : posInt(pod, "Poddělení parcely"),
  };
}

/** Builds the ident to save; throws Error with a Czech message when the form is incomplete. */
export function identFromForm(f: IdentForm): ItemIdent {
  if (f.ku === null) throw new Error("Vyberte katastrální území ze seznamu.");
  if (f.kind === "parcela") {
    return { Parcela: { ku: f.ku, parcela: parcelaFromForm(f.parcela) } };
  }
  const cislo_domovni = posInt(f.cisloDomovni, "Číslo domovní");
  if (f.kind === "stavba") {
    return { Stavba: { ku: f.ku, typ: f.typ, cislo_domovni, na_parcele: parcelaFromForm(f.parcela) } };
  }
  return {
    Jednotka: {
      ku: f.ku,
      typ: f.typ,
      cislo_domovni,
      cislo_jednotky: posInt(f.cisloJednotky, "Číslo jednotky"),
      kod_casti_obce: f.kodCastiObce,
    },
  };
}
