export * from "../../generated/subscription-payments.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.SUBSCRIPTION_PAYMENTS_CONTRACT_ID) ||
      "CB7F22YZ2A7QLB3Z53Y3UUXBDPR5I5ZZTNFOP3OGKZQEIOPGDGIT3AK4",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
