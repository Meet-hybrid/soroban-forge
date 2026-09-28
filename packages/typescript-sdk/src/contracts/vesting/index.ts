export * from "../../generated/vesting.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.VESTING_CONTRACT_ID) ||
      "CCF5GTS7RMFXE6FI3VHVGIPNLCYNNAWVDFJJN4XQSVE6YEBFVNJDN56W",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
