import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const sdkDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const rootDir = resolve(sdkDir, "../..");
const manifestPath = join(sdkDir, "generated-client-manifest.json");

/**
 * Calculate SHA-256 checksum of a file
 */
function calculateChecksum(filePath) {
  const content = readFileSync(filePath);
  return createHash("sha256").update(content).digest("hex");
}

/**
 * Calculate checksums for all files in a directory recursively
 */
function calculateDirectoryChecksums(dirPath) {
  const checksums = {};
  
  function traverse(currentPath, relativePath = "") {
    const entries = readdirSync(currentPath);
    
    for (const entry of entries) {
      const fullPath = join(currentPath, entry);
      const relPath = relativePath ? join(relativePath, entry) : entry;
      const stat = statSync(fullPath);
      
      if (stat.isDirectory()) {
        traverse(fullPath, relPath);
      } else if (stat.isFile()) {
        checksums[relPath] = calculateChecksum(fullPath);
      }
    }
  }
  
  traverse(dirPath);
  return checksums;
}

/**
 * Get the current Git commit SHA
 */
function getGitCommit() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: rootDir,
    encoding: "utf8",
  });
  
  if (result.error) {
    throw new Error(`Failed to get git commit: ${result.error.message}`);
  }
  
  if (result.status !== 0) {
    throw new Error(`git rev-parse failed with status ${result.status}: ${result.stderr}`);
  }
  
  return result.stdout.trim();
}

/**
 * Verify that a Git commit exists in the repository
 */
function verifyGitCommitExists(commit) {
  const result = spawnSync("git", ["cat-file", "-e", commit], {
    cwd: rootDir,
  });
  
  return result.status === 0;
}

/**
 * Get version from a Cargo.toml file
 */
function getCargoVersion(crateName) {
  const cargoPath = join(rootDir, "crates", crateName.replace("soroban-forge-", ""), "Cargo.toml");
  
  if (!existsSync(cargoPath)) {
    throw new Error(`Cargo.toml not found for crate ${crateName} at ${cargoPath}`);
  }
  
  const content = readFileSync(cargoPath, "utf8");
  
  // Check for workspace version first
  if (content.includes("version.workspace = true")) {
    const workspaceCargoPath = join(rootDir, "Cargo.toml");
    const workspaceContent = readFileSync(workspaceCargoPath, "utf8");
    const versionMatch = workspaceContent.match(/^\s*version\s*=\s*"([^"]+)"/m);
    
    if (versionMatch) {
      return versionMatch[1];
    }
  }
  
  // Check for direct version
  const versionMatch = content.match(/^\[package\][^\[]*version\s*=\s*"([^"]+)"/ms);
  if (versionMatch) {
    return versionMatch[1];
  }
  
  throw new Error(`Could not determine version for ${crateName}`);
}

/**
 * Verify the generated-client manifest
 */
function verifyManifest() {
  console.log("==> Verifying generated-client manifest");
  
  // Check manifest exists
  if (!existsSync(manifestPath)) {
    console.error(`ERROR: Manifest not found at ${manifestPath}`);
    process.exit(1);
  }
  
  // Parse manifest
  let manifest;
  try {
    const manifestContent = readFileSync(manifestPath, "utf8");
    manifest = JSON.parse(manifestContent);
  } catch (error) {
    console.error(`ERROR: Failed to parse manifest: ${error.message}`);
    process.exit(1);
  }
  
  // Validate manifest schema
  if (!manifest.schema || manifest.schema !== "soroban-forge-generated-clients-v1") {
    console.error(`ERROR: Invalid or unsupported manifest schema: ${manifest.schema}`);
    process.exit(1);
  }
  
  if (!manifest.generatedAt || !manifest.generationCommit) {
    console.error("ERROR: Manifest missing required fields (generatedAt, generationCommit)");
    process.exit(1);
  }
  
  if (!Array.isArray(manifest.contracts) || manifest.contracts.length === 0) {
    console.error("ERROR: Manifest contracts array is missing or empty");
    process.exit(1);
  }
  
  // Verify generation commit exists
  if (!verifyGitCommitExists(manifest.generationCommit)) {
    console.error(`ERROR: Generation commit ${manifest.generationCommit} does not exist in repository`);
    process.exit(1);
  }
  
  console.log(`  Manifest schema: ${manifest.schema}`);
  console.log(`  Generated at: ${manifest.generatedAt}`);
  console.log(`  Generation commit: ${manifest.generationCommit}`);
  console.log(`  Contracts: ${manifest.contracts.length}`);
  
  let hasErrors = false;
  
  // Verify each contract entry
  for (const contract of manifest.contracts) {
    console.log(`\n  Verifying ${contract.clientModule}...`);
    
    // Validate required fields
    if (!contract.crateName || !contract.clientModule || !contract.contractVersion) {
      console.error(`    ERROR: Missing required fields for ${contract.clientModule}`);
      hasErrors = true;
      continue;
    }
    
    // Verify crate version matches Cargo.toml
    try {
      const actualVersion = getCargoVersion(contract.crateName);
      if (actualVersion !== contract.contractVersion) {
        console.error(`    ERROR: Version mismatch for ${contract.crateName}`);
        console.error(`      Manifest: ${contract.contractVersion}`);
        console.error(`      Cargo.toml: ${actualVersion}`);
        hasErrors = true;
        continue;
      }
      console.log(`    Contract version: ${actualVersion} ✓`);
    } catch (error) {
      console.error(`    ERROR: ${error.message}`);
      hasErrors = true;
      continue;
    }
    
    // Verify generated files exist and checksums match
    if (!contract.generatedFiles || Object.keys(contract.generatedFiles).length === 0) {
      console.error(`    ERROR: No generated files recorded for ${contract.clientModule}`);
      hasErrors = true;
      continue;
    }
    
    for (const [relPath, expectedChecksum] of Object.entries(contract.generatedFiles)) {
      const fullPath = join(sdkDir, relPath);
      
      if (!existsSync(fullPath)) {
        console.error(`    ERROR: Generated file missing: ${relPath}`);
        hasErrors = true;
        continue;
      }
      
      const actualChecksum = calculateChecksum(fullPath);
      if (actualChecksum !== expectedChecksum) {
        console.error(`    ERROR: Checksum mismatch for ${relPath}`);
        console.error(`      Expected: ${expectedChecksum}`);
        console.error(`      Actual: ${actualChecksum}`);
        hasErrors = true;
      }
    }
    
    console.log(`    Generated files: ${Object.keys(contract.generatedFiles).length} ✓`);
    
    // Verify standalone package if present
    if (contract.standalonePackage) {
      const packageDir = join(rootDir, "packages", contract.standalonePackage);
      if (!existsSync(packageDir)) {
        console.error(`    ERROR: Standalone package directory missing: ${contract.standalonePackage}`);
        hasErrors = true;
      } else {
        console.log(`    Standalone package: ${contract.standalonePackage} ✓`);
      }
    }
  }
  
  if (hasErrors) {
    console.error("\n❌ Manifest verification FAILED");
    process.exit(1);
  }
  
  console.log("\n✓ Manifest verification PASSED");
}

verifyManifest();
