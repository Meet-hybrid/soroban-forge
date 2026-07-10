export class SorobanForgeSDK {
  constructor(private network: string, private rpcUrl: string) {}

  async connect() {
    console.log(`Connected to ${this.network} at ${this.rpcUrl}`);
  }

  async escrow() {
    return {
      create: async () => console.log('create escrow'),
      deposit: async () => console.log('deposit'),
      release: async () => console.log('release'),
      refund: async () => console.log('refund'),
    };
  }
}
