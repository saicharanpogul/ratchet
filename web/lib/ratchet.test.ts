/**
 * Vitest end-to-end test: imports the `--target nodejs` wasm-pack
 * output (built by the `pretest` script) and exercises `check_upgrade`
 * through the same JS-glue-over-wasm-bindgen stack the browser uses,
 * minus the `--target web` fetch() bit that Node can't resolve for
 * local file URLs.
 *
 * The browser-target module is covered indirectly: both outputs
 * compile from the same `lib.rs`, so a pass here plus a green
 * `wasm-pack test --node` confirms the rule engine works under every
 * target this repo ships.
 */

import { describe, expect, it } from "vitest";
import {
  check_readiness,
  check_upgrade,
  version,
} from "./ratchet-wasm-node/solana_ratchet_wasm.js";

type Severity = "additive" | "unsafe" | "breaking";
interface Finding {
  rule_id: string;
  rule_name: string;
  severity: Severity;
  path: string[];
  message: string;
}
interface Report {
  findings: Finding[];
}

function checkUpgrade(oldIdl: string, newIdl: string): Report {
  const json = check_upgrade(oldIdl, newIdl);
  return JSON.parse(json) as Report;
}

function ratchetVersion(): string {
  return version();
}

const V1 = JSON.stringify({
  metadata: { name: "vault" },
  instructions: [
    {
      name: "deposit",
      discriminator: [242, 35, 198, 137, 82, 225, 242, 182],
      accounts: [
        { name: "user", signer: true },
        { name: "vault", writable: true },
      ],
      args: [{ name: "amount", type: "u64" }],
    },
    {
      name: "withdraw",
      discriminator: [8, 7, 6, 5, 4, 3, 2, 1],
      accounts: [{ name: "user" }],
      args: [],
    },
  ],
  accounts: [
    { name: "Vault", discriminator: [211, 8, 232, 43, 2, 152, 117, 119] },
  ],
  types: [
    {
      name: "Vault",
      type: {
        kind: "struct",
        fields: [
          { name: "owner", type: "pubkey" },
          { name: "balance", type: "u64" },
        ],
      },
    },
  ],
});

const V2_BREAKING = JSON.stringify({
  metadata: { name: "vault" },
  instructions: [
    {
      name: "deposit",
      discriminator: [242, 35, 198, 137, 82, 225, 242, 182],
      accounts: [
        { name: "user", signer: true },
        { name: "vault", writable: true },
      ],
      args: [{ name: "amount", type: "u32" }],
    },
  ],
  accounts: [
    { name: "Vault", discriminator: [99, 99, 99, 99, 99, 99, 99, 99] },
  ],
  types: [
    {
      name: "Vault",
      type: {
        kind: "struct",
        fields: [
          { name: "balance", type: "u64" },
          { name: "owner", type: "pubkey" },
        ],
      },
    },
  ],
});

describe("ratchet-wasm integration", () => {
  it("reports the crate version", () => {
    const v = ratchetVersion();
    expect(v).toMatch(/^\d+\.\d+\.\d+/);
  });

  it("returns an empty report for identical IDLs", () => {
    const r = checkUpgrade(V1, V1);
    expect(r.findings).toEqual([]);
  });

  it("fires R001, R006, R007, R008 on the breaking vault diff", () => {
    const r = checkUpgrade(V1, V2_BREAKING);
    const ids = new Set(r.findings.map((f) => f.rule_id));
    // Field reorder, discriminator change, instruction removed,
    // instruction arg type change.
    expect(ids).toContain("R001");
    expect(ids).toContain("R006");
    expect(ids).toContain("R007");
    expect(ids).toContain("R008");
  });

  it("returns findings typed as the Report shape", () => {
    const r = checkUpgrade(V1, V2_BREAKING);
    for (const f of r.findings) {
      expect(["additive", "unsafe", "breaking"]).toContain(f.severity);
      expect(f.rule_id).toMatch(/^R\d{3}$/);
      expect(Array.isArray(f.path)).toBe(true);
      expect(typeof f.message).toBe("string");
    }
  });

  it("surfaces a JS error on malformed input", () => {
    expect(() => checkUpgrade("not json", V1)).toThrow();
  });

  it("readiness fires preflight rules on a bare V1 surface", () => {
    const json = check_readiness(V1);
    const report = JSON.parse(json) as Report;
    const ids = new Set(report.findings.map((f) => f.rule_id));
    // V1 has no version field (P001) and no reserved padding (P002).
    expect(ids).toContain("P001");
    expect(ids).toContain("P002");
  });
});
