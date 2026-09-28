export * from "../../generated/multi-sig-wallet.js";

export const networks = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    contractId:
      (typeof process !== "undefined" && process.env?.MULTI_SIG_WALLET_CONTRACT_ID) ||
      "CDPWOO6XPI5QCXAJD6XRGO74P645D4VCJK33YH5XYEJJRO3IP4L7SGSN",
  },
} as const;

export const config = {
  contractId: networks.testnet.contractId,
  networkPassphrase: networks.testnet.networkPassphrase,
} as const;
