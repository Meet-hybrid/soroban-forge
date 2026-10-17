/** @type {import('next').NextConfig} */
import path from "node:path";

const nextConfig = {
  reactStrictMode: true,
  transpilePackages: ["@soroban-forge/escrow-client"],
  turbopack: {
    root: path.resolve(__dirname, ".."),
  },
};

export default nextConfig
