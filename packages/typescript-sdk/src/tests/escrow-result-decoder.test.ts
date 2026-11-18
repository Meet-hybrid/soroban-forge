import test from "node:test";
import assert from "node:assert/strict";

import { decodeEscrowResult } from "../index.js";

test("decodeEscrowResult normalizes a successful escrow status result", () => {
  const decoded = decodeEscrowResult({
    ok: true,
    value: { tag: "Funded", values: null },
  });

  assert.deepEqual(decoded, {
    ok: true,
    value: { tag: "Funded", values: null },
  });
});

test("decodeEscrowResult normalizes a contract error result", () => {
  const decoded = decodeEscrowResult({
    ok: false,
    error: { code: 3, message: "InvalidInput" },
  });

  assert.deepEqual(decoded, {
    ok: false,
    error: {
      code: 3,
      message: "InvalidInput",
      raw: { code: 3, message: "InvalidInput" },
    },
  });
});

test("decodeEscrowResult handles malformed payloads gracefully", () => {
  const decoded = decodeEscrowResult("not-a-result");

  assert.deepEqual(decoded, {
    ok: false,
    error: {
      message: "Malformed escrow result payload",
      raw: "not-a-result",
    },
  });
});
