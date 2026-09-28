export * from "../../generated/escrow.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.ESCROW_CONTRACT_ID) ||
      "CC227UDF6WBLRTOKKVRIJN7BGSBK67ZGV6IDARJ2AMATGSQ7UZNBZHSB",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
