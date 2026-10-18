import { createRequire } from "module";
import path from "path";
import nextConfig from "eslint-config-next";

// ESLint 10 compatibility patches for Next.js / legacy plugins (eslint-plugin-react):
// 1. Polyfill removed RuleContext methods on FileContext prototype
try {
  const require = createRequire(import.meta.url);
  const eslintPath = require.resolve("eslint/package.json");
  const fileContextPath = path.join(path.dirname(eslintPath), "lib/linter/file-context.js");
  const { FileContext } = require(fileContextPath);

  if (FileContext && FileContext.prototype) {
    if (!FileContext.prototype.getFilename) {
      FileContext.prototype.getFilename = function () {
        return this.filename;
      };
    }
    if (!FileContext.prototype.getPhysicalFilename) {
      FileContext.prototype.getPhysicalFilename = function () {
        return this.physicalFilename;
      };
    }
    if (!FileContext.prototype.getSourceCode) {
      FileContext.prototype.getSourceCode = function () {
        return this.sourceCode;
      };
    }
    if (!FileContext.prototype.getCwd) {
      FileContext.prototype.getCwd = function () {
        return this.cwd;
      };
    }
    if (!FileContext.prototype.getScope) {
      FileContext.prototype.getScope = function () {
        return this.sourceCode.getScope(this.sourceCode.ast);
      };
    }
    if (!FileContext.prototype.getDeclaredVariables) {
      FileContext.prototype.getDeclaredVariables = function (node) {
        return this.sourceCode.getDeclaredVariables(node);
      };
    }
    if (!FileContext.prototype.markVariableAsUsed) {
      FileContext.prototype.markVariableAsUsed = function (name) {
        return this.sourceCode.markVariableAsUsed(name);
      };
    }
  }
} catch (e) {
  // If resolution fails, continue without throwing
}

// 2. Wrap parser's parseForESLint to provide addGlobals expected by ESLint 10
const patchedConfig = nextConfig.map((cfg) => {
  if (cfg.languageOptions?.parser?.parseForESLint) {
    const origParseForESLint = cfg.languageOptions.parser.parseForESLint;
    const patchedParser = {
      ...cfg.languageOptions.parser,
      parseForESLint(...args) {
        const result = origParseForESLint.apply(this, args);
        if (result && result.scopeManager && typeof result.scopeManager.addGlobals !== "function") {
          result.scopeManager.addGlobals = function (names) {
            const globalScope = this.scopes[0];
            if (!globalScope) return;
            if (typeof globalScope.__addVariables === "function") {
              globalScope.__addVariables(names);
            } else if (typeof globalScope.__defineGeneric === "function") {
              for (const name of names) {
                if (!globalScope.set.has(name)) {
                  globalScope.__defineGeneric(name, globalScope.set, globalScope.variables, null, null);
                }
              }
            }
          };
        }
        return result;
      },
    };
    return {
      ...cfg,
      languageOptions: {
        ...cfg.languageOptions,
        parser: patchedParser,
      },
    };
  }
  return cfg;
});

export default patchedConfig;
import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

export default defineConfig([
  ...nextVitals,
  ...nextTypescript,
  globalIgnores([".next/**", "next-env.d.ts"]),
]);
