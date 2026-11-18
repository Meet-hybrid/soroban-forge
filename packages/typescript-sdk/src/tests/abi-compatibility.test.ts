import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { xdr } from "@stellar/stellar-sdk";
import {
  dao,
  escrow,
  marketplace,
  multisig,
  subscription,
  vesting,
} from "../../dist/index.js";
import type {
  dao as DaoTypes,
  escrow as EscrowTypes,
  marketplace as MarketplaceTypes,
  multisig as MultisigTypes,
  subscription as SubscriptionTypes,
  vesting as VestingTypes,
} from "../../dist/index.js";
import { FORGE_ERRORS } from "../errors.generated.js";

const CONTRACT_ID =
  "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB";
const CLIENT_OPTIONS = {
  contractId: CONTRACT_ID,
  networkPassphrase: "Test SDF Network ; September 2015",
  rpcUrl: "https://localhost",
};

type FunctionAbi = {
  name: string;
  inputs: Array<{ name: string; type: string }>;
  outputs: string[];
};

type EventAbi = {
  name: string;
  dataFormat: string;
  params: Array<{ name: string; type: string }>;
};

type TypeAbi =
  | {
      kind: "struct";
      name: string;
      fields: Array<{ name: string; type: string }>;
    }
  | {
      kind: "enum" | "error-enum";
      name: string;
      variants: Array<{ name: string; value: number }>;
    }
  | {
      kind: "union";
      name: string;
      variants: Array<{ name: string; types: string[] }>;
    };

type AbiFixture = {
  schema: "soroban-forge/contract-abi@1";
  contract: string;
  functions: FunctionAbi[];
  events: EventAbi[];
  types: TypeAbi[];
  errors: Array<{ name: string; code: number }>;
};

function compareName(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function typeName(type: xdr.ScSpecTypeDef): string {
  const simpleName = type.switch().name.replace(/^scSpecType/, "");

  if (simpleName === "Udt") {
    return `udt:${type.udt().name().toString()}`;
  }
  if (simpleName === "Vec") {
    return `vec<${typeName(type.vec().elementType())}>`;
  }
  if (simpleName === "Option") {
    return `option<${typeName(type.option().valueType())}>`;
  }
  if (simpleName === "Result") {
    const result = type.result();
    return `result<${typeName(result.okType())},${typeName(result.errorType())}>`;
  }
  if (simpleName === "Map") {
    const map = type.map();
    return `map<${typeName(map.keyType())},${typeName(map.valueType())}>`;
  }
  if (simpleName === "Tuple") {
    return `tuple<${type.tuple().valueTypes().map(typeName).join(",")}>`;
  }
  return simpleName[0]!.toLowerCase() + simpleName.slice(1);
}

function normalizeAbi(spec: InstanceType<typeof escrow.Client>["spec"]) {
  const functions: FunctionAbi[] = spec.funcs().map((fn) => ({
    name: fn.name().toString(),
    inputs: fn.inputs().map((input) => ({
      name: input.name().toString(),
      type: typeName(input.type()),
    })),
    outputs: fn.outputs().map(typeName),
  }));
  const events: EventAbi[] = spec.events().map((event) => ({
    name: event.name().toString(),
    dataFormat: event.dataFormat().name,
    params: event.params().map((param) => ({
      name: param.name().toString(),
      type: typeName(param.type()),
    })),
  }));
  const types: TypeAbi[] = spec.entries
    .filter((entry) => entry.switch().name.startsWith("scSpecEntryUdt"))
    .map((entry) => {
      const kind = entry.switch().name;

      if (kind === "scSpecEntryUdtStructV0") {
        const udt = entry.udtStructV0();
        return {
          kind: "struct" as const,
          name: udt.name().toString(),
          fields: udt.fields().map((field) => ({
            name: field.name().toString(),
            type: typeName(field.type()),
          })),
        };
      }
      if (
        kind === "scSpecEntryUdtEnumV0" ||
        kind === "scSpecEntryUdtErrorEnumV0"
      ) {
        const udt =
          kind === "scSpecEntryUdtErrorEnumV0"
            ? entry.udtErrorEnumV0()
            : entry.udtEnumV0();
        return {
          kind: kind === "scSpecEntryUdtErrorEnumV0" ? "error-enum" as const : "enum" as const,
          name: udt.name().toString(),
          variants: udt.cases().map((variant) => ({
            name: variant.name().toString(),
            value: Number(variant.value().toString()),
          })),
        };
      }
      if (kind === "scSpecEntryUdtUnionV0") {
        const udt = entry.udtUnionV0();
        return {
          kind: "union" as const,
          name: udt.name().toString(),
          variants: udt.cases().map((variant) => {
            const variantKind = variant.switch().name;
            if (variantKind === "scSpecUdtUnionCaseVoidV0") {
              return { name: variant.voidCase().name().toString(), types: [] };
            }
            if (variantKind === "scSpecUdtUnionCaseTupleV0") {
              const tupleCase = variant.tupleCase();
              return {
                name: tupleCase.name().toString(),
                types: tupleCase.type().map(typeName),
              };
            }
            throw new Error(`Unsupported union variant ${variantKind} in ${udt.name()}`);
          }),
        };
      }
      throw new Error(`Unsupported contract spec entry ${kind}`);
    });
  const errors = spec.errorCases().map((error) => ({
    name: error.name().toString(),
    code: Number(error.value().toString()),
  }));

  functions.sort((left, right) => compareName(left.name, right.name));
  events.sort((left, right) => compareName(left.name, right.name));
  types.sort((left, right) => compareName(left.name, right.name));
  errors.sort((left, right) => left.code - right.code);
  return { functions, events, types, errors };
}

const clients = {
  escrow: new escrow.Client(CLIENT_OPTIONS),
  vesting: new vesting.Client(CLIENT_OPTIONS),
  multisig: new multisig.Client(CLIENT_OPTIONS),
  dao: new dao.Client(CLIENT_OPTIONS),
  subscription: new subscription.Client(CLIENT_OPTIONS),
  marketplace: new marketplace.Client(CLIENT_OPTIONS),
} as const;

for (const [module, client] of Object.entries(clients)) {
  test(`${module}: ABI, events, enums, returns, and errors match its fixture`, () => {
    const fixturePath = resolve(
      "src",
      "abi-fixtures",
      `${module}.json`,
    );
    const fixture = JSON.parse(
      readFileSync(fixturePath, "utf8"),
    ) as AbiFixture;
    assert.equal(fixture.schema, "soroban-forge/contract-abi@1");
    assert.deepEqual(normalizeAbi(client.spec), {
      functions: fixture.functions,
      events: fixture.events,
      types: fixture.types,
      errors: fixture.errors,
    });

    for (const error of fixture.errors) {
      assert.equal(
        FORGE_ERRORS[error.code],
        error.name,
        `${module}: ForgeError code ${error.code} must remain ${error.name}`,
      );
    }
  });
}

// These aliases compile only if every generated contract namespace keeps its
// public domain types exported from the unified SDK entrypoint.
type _PublicContractTypes = [
  EscrowTypes.EscrowData,
  EscrowTypes.EscrowStatus,
  EscrowTypes.ParticipantEscrowsPage,
  VestingTypes.VestingSchedule,
  VestingTypes.VestingStatus,
  MultisigTypes.WalletTx,
  MultisigTypes.TxKind,
  MultisigTypes.TxStatus,
  MultisigTypes.Withdrawal,
  MultisigTypes.LimitChange,
  MultisigTypes.WithdrawalLimit,
  DaoTypes.Proposal,
  DaoTypes.ProposalState,
  DaoTypes.BondState,
  DaoTypes.BondConfig,
  MarketplaceTypes.Royalty,
  MarketplaceTypes.RoyaltyStatus,
  MarketplaceTypes.Settlement,
  MarketplaceTypes.SettlementSummary,
  SubscriptionTypes.Subscription,
  SubscriptionTypes.SubscriptionStatus,
];

const _escrowStatus: EscrowTypes.EscrowStatus = {
  tag: "Pending",
  values: undefined,
};
const _vestingStatus: VestingTypes.VestingStatus = {
  tag: "Locked",
  values: undefined,
};
const _multisigStatus: MultisigTypes.TxStatus = {
  tag: "Pending",
  values: undefined,
};
const _proposalState: DaoTypes.ProposalState = {
  tag: "Active",
  values: undefined,
};
const _royaltyStatus: MarketplaceTypes.RoyaltyStatus = {
  tag: "Active",
  values: undefined,
};
const _subscriptionStatus: SubscriptionTypes.SubscriptionStatus = {
  tag: "Active",
  values: undefined,
};
void _escrowStatus;
void _vestingStatus;
void _multisigStatus;
void _proposalState;
void _royaltyStatus;
void _subscriptionStatus;
