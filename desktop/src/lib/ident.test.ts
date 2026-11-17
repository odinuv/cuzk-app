import { describe, expect, it } from "vitest";
import { emptyIdentForm, identFromForm, identToForm } from "./ident";

describe("ident form mapping", () => {
  it("round-trips a parcela", () => {
    const ident = { Parcela: { ku: 610003, parcela: { druh: "Pozemkova" as const, kmenove: 3756, poddeleni: 2 } } };
    expect(identFromForm(identToForm(ident))).toEqual(ident);
  });
  it("round-trips a stavba and a jednotka", () => {
    const s = { Stavba: { ku: 1, typ: "Ce" as const, cislo_domovni: 3, na_parcele: { druh: "Stavebni" as const, kmenove: 9, poddeleni: null } } };
    expect(identFromForm(identToForm(s))).toEqual(s);
    const j = { Jednotka: { ku: 1, typ: "Cp" as const, cislo_domovni: 5, cislo_jednotky: 12, kod_casti_obce: 77 } };
    expect(identFromForm(identToForm(j))).toEqual(j);
  });
  it("requires a k.ú. and valid numbers", () => {
    const f = emptyIdentForm();
    expect(() => identFromForm(f)).toThrow(/katastrální území/);
    f.ku = 1;
    expect(() => identFromForm(f)).toThrow(/Kmenové číslo/);
    f.parcela.kmenove = "12";
    f.parcela.poddeleni = "x";
    expect(() => identFromForm(f)).toThrow(/Poddělení/);
    f.parcela.poddeleni = "";
    expect(identFromForm(f)).toEqual({ Parcela: { ku: 1, parcela: { druh: "Pozemkova", kmenove: 12, poddeleni: null } } });
  });
});
