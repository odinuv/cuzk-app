import { describe, expect, it } from "vitest";
import { formatChange, formatDate, detailRows, formatDue, formatRizeni, formatRunSummary, formatValue, statusLabel } from "./format";

describe("formatChange", () => {
  it("renders path: before → after", () => {
    expect(formatChange({ path: "zpusobVyuziti", before: "orná půda", after: "zahrada" })).toBe(
      "zpusobVyuziti: orná půda → zahrada",
    );
  });
  it("uses — for missing values", () => {
    expect(formatChange({ path: "a", before: null, after: 5 })).toBe("a: — → 5");
    expect(formatChange({ path: "a", before: 5, after: null })).toBe("a: 5 → —");
    expect(formatChange({ path: "a", before: undefined as never, after: "" })).toBe("a: — → —");
  });
  it("keeps nested paths", () => {
    expect(formatChange({ path: "stavba.cislaDomovni[0].cislo", before: 1, after: 2 })).toBe(
      "stavba.cislaDomovni[0].cislo: 1 → 2",
    );
  });
  it("renders objects, arrays and booleans readably", () => {
    expect(formatChange({ path: "x", before: { a: 1 }, after: [1, 2] })).toBe('x: {"a":1} → [1,2]');
    expect(formatChange({ path: "x", before: true, after: false })).toBe("x: ano → ne");
  });
});

describe("formatValue", () => {
  it("handles empty array as —", () => {
    expect(formatValue([])).toBe("—");
  });
});

describe("formatRizeni", () => {
  it("formats V-1234/2026-701", () => {
    expect(
      formatRizeni({ id: 9, typRizeni: "V", poradoveCislo: 1234, rok: 2026, kodPracoviste: 701 }),
    ).toBe("V-1234/2026-701");
  });
});

describe("statusLabel", () => {
  it("maps all five statuses", () => {
    expect(statusLabel("First")).toBe("první kontrola");
    expect(statusLabel("Unchanged")).toBe("beze změn");
    expect(statusLabel("Changed")).toBe("změna");
    expect(statusLabel("Error")).toBe("chyba");
    expect(statusLabel("Deferred")).toBe("čeká na další den");
  });
});

describe("formatDate", () => {
  it("formats Czech date and handles missing", () => {
    expect(formatDate("2026-11-17T12:00:00Z")).toBe("17. 11. 2026");
    expect(formatDate(null)).toBe("—");
    expect(formatDate("nesmysl")).toBe("—");
  });
});

describe("formatDue", () => {
  it("renders the overview line", () => {
    expect(formatDue({ dueNow: 5, waiting: 0, remainingCalls: 412 })).toBe(
      "Na řadě: 5 · čeká: 0 · zbývá volání dnes: 412",
    );
    expect(formatDue({ dueNow: 1, waiting: 2, remainingCalls: null })).toBe(
      "Na řadě: 1 · čeká: 2 · zbývá volání dnes: —",
    );
  });
});

describe("formatRunSummary", () => {
  const base = {
    runId: 1, status: "ok" as const, checked: 4, changed: 0, plombyNove: 0, errors: 0,
    deferred: 0, apiCalls: 5, apiUsedToday: 5, apiLimit: 500, aktualnostDat: null, error: null,
  };
  it("lists only non-zero counts", () => {
    expect(formatRunSummary(base)).toBe("zkontrolováno 4");
    expect(formatRunSummary({ ...base, changed: 2, plombyNove: 1, errors: 1, deferred: 3 })).toBe(
      "zkontrolováno 4 · změn 2 · nových plomb 1 · chyb 1 · odloženo na další den 3",
    );
  });
  it("shows the error of a failed run", () => {
    expect(formatRunSummary({ ...base, status: "failed", error: "Kontrola už běží." })).toBe("Kontrola už běží.");
  });
});

describe("detailRows", () => {
  it("flattens nested objects and arrays with depth", () => {
    expect(detailRows({ a: 1, b: { c: "x", d: null }, e: [{ f: true }], g: [] })).toEqual([
      { depth: 0, key: "a", value: "1" },
      { depth: 0, key: "b", value: null },
      { depth: 1, key: "c", value: "x" },
      { depth: 1, key: "d", value: "—" },
      { depth: 0, key: "e", value: null },
      { depth: 1, key: "[1]", value: null },
      { depth: 2, key: "f", value: "ano" },
      { depth: 0, key: "g", value: "—" },
    ]);
  });
  it("returns no rows for a scalar", () => {
    expect(detailRows(5)).toEqual([]);
  });
});
