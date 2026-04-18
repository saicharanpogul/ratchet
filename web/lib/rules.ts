export interface RuleSpec {
  id: string;
  name: string;
  severity: "BREAKING" | "UNSAFE" | "ADDITIVE" | "MIXED";
  allow: string | null;
  description: string;
}

/**
 * Mirror of the ratchet rule catalog. Keep in sync with
 * `crates/ratchet-core/src/rules/r*.rs` module-level docs.
 */
export const RULES: RuleSpec[] = [
  {
    id: "R001",
    name: "account-field-reorder",
    severity: "BREAKING",
    allow: null,
    description:
      "Shared account fields were reordered; every existing account now deserializes to garbage because Borsh lays fields out by declaration order.",
  },
  {
    id: "R002",
    name: "account-field-retype",
    severity: "BREAKING",
    allow: "allow-type-change",
    description:
      "A shared field's type changed. Any size change shifts every later byte offset; same-size retypes (u64 → i64) are wire-compatible but semantic breaks.",
  },
  {
    id: "R003",
    name: "account-field-removed",
    severity: "BREAKING",
    allow: "allow-field-removed / --migrated-account",
    description:
      "A field was removed from an account. Its bytes remain on-chain and get misread as the next field by the new program.",
  },
  {
    id: "R004",
    name: "account-field-insert-middle",
    severity: "BREAKING",
    allow: "allow-field-insert / --migrated-account",
    description:
      "A new field was inserted before existing fields. Borsh offsets shift, corrupting every existing account.",
  },
  {
    id: "R005",
    name: "account-field-append",
    severity: "UNSAFE",
    allow: "allow-field-append / --realloc-account / --migrated-account",
    description:
      "A new field was appended. Existing accounts lack those bytes; they need reallocation via Anchor's realloc constraint or a migration.",
  },
  {
    id: "R006",
    name: "account-discriminator-change",
    severity: "BREAKING",
    allow: "allow-rename",
    description:
      "An account's 8-byte discriminator changed (typically a struct rename). Every existing on-chain account fails AccountDiscriminatorMismatch.",
  },
  {
    id: "R007",
    name: "instruction-removed",
    severity: "BREAKING",
    allow: "allow-ix-removal",
    description:
      "An instruction was removed. Every existing client calling it gets InstructionFallbackNotFound.",
  },
  {
    id: "R008",
    name: "instruction-arg-change",
    severity: "BREAKING",
    allow: "allow-ix-arg-change",
    description:
      "An instruction's argument signature changed (reordered, retyped, added, removed). Existing clients send bytes the program misreads.",
  },
  {
    id: "R009",
    name: "instruction-account-list-change",
    severity: "BREAKING",
    allow: "allow-ix-account-change",
    description:
      "An instruction's account list shape changed. Solana dispatches accounts by index; the wrong account now lands at each slot's position.",
  },
  {
    id: "R010",
    name: "instruction-signer-writable-flip",
    severity: "MIXED",
    allow: "allow-signer-mut-flip",
    description:
      "is_signer or is_writable toggled on an existing slot. Tightening (false → true) breaks existing callers; relaxation (true → false) is safe.",
  },
  {
    id: "R011",
    name: "enum-variant-removed-or-inserted",
    severity: "BREAKING",
    allow: null,
    description:
      "A Borsh-serialized enum variant was removed or inserted before existing variants, shifting the ordinal of every later variant.",
  },
  {
    id: "R012",
    name: "enum-variant-append",
    severity: "ADDITIVE",
    allow: null,
    description:
      "A new variant was appended at the tail. Ordinals of existing variants are preserved — safe; reported for visibility only.",
  },
  {
    id: "R013",
    name: "pda-seed-change",
    severity: "BREAKING",
    allow: "allow-pda-shape-change (presence flip only)",
    description:
      "A PDA's seed expression changed. Every existing account at the old address is orphaned; no client can re-derive it with the new formula.",
  },
  {
    id: "R014",
    name: "instruction-discriminator-change",
    severity: "BREAKING",
    allow: "allow-ix-rename",
    description:
      "An instruction's discriminator changed (handler rename, explicit override). Existing callers dispatch into the wrong handler.",
  },
  {
    id: "R015",
    name: "account-removed",
    severity: "BREAKING",
    allow: "allow-account-removal",
    description:
      "An account struct disappeared entirely. Every existing on-chain account of that type is unreachable through the new program.",
  },
  {
    id: "R016",
    name: "event-discriminator-change",
    severity: "BREAKING",
    allow: "allow-event-rename",
    description:
      "An event's 8-byte log selector changed. Every off-chain indexer filtering for the old value goes silent on the next emit.",
  },
];
