export * from "../../generated/dao-governance.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.DAO_GOVERNANCE_CONTRACT_ID) ||
      "CCX2VGJ5ASRHPFQ4O2CIQC5P564I7IYGORVZWVRH7JGOVSYI74557OWY",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
