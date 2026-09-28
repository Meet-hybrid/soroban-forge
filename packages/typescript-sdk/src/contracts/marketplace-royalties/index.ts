export * from "../../generated/marketplace-royalties.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.MARKETPLACE_ROYALTIES_CONTRACT_ID) ||
      "CDCWAWC47TUKWYQ3TYMD2KFED67B2GKJE7F2IHMD5PSWCDTI644NP4RN",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
