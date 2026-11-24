#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    const MATRIX: &str = include_str!("../../../../docs/contracts/authorization-matrix.md");
    const CONTRACTS_INDEX: &str = include_str!("../../../../docs/contracts/index.md");
    const CONTRACTS_README: &str = include_str!("../../../../docs/contracts/README.md");
    const DOCS_INDEX: &str = include_str!("../../../../docs/README.md");
    const ERRORS_SOURCE: &str = include_str!("../../../../crates/shared-utils/src/errors.rs");

    struct ContractDoc {
        heading: &'static str,
        crate_path: &'static str,
        package: &'static str,
        client_path: &'static str,
        standalone_client: Option<&'static str>,
    }

    const CONTRACTS: &[ContractDoc] = &[
        ContractDoc {
            heading: "Escrow",
            crate_path: "crates/escrow",
            package: "soroban-forge-escrow",
            client_path: "packages/typescript-sdk/src/clients/escrow.ts",
            standalone_client: None,
        },
        ContractDoc {
            heading: "Vesting",
            crate_path: "crates/vesting",
            package: "soroban-forge-vesting",
            client_path: "packages/typescript-sdk/src/clients/vesting.ts",
            standalone_client: Some("packages/vesting-client/src/index.ts"),
        },
        ContractDoc {
            heading: "Multi-Sig Wallet",
            crate_path: "crates/multi-sig-wallet",
            package: "soroban-forge-multi-sig-wallet",
            client_path: "packages/typescript-sdk/src/clients/multisig.ts",
            standalone_client: Some("packages/multi-sig-wallet-client/src/index.ts"),
        },
        ContractDoc {
            heading: "DAO Governance",
            crate_path: "crates/dao-governance",
            package: "soroban-forge-dao-governance",
            client_path: "packages/typescript-sdk/src/clients/dao.ts",
            standalone_client: Some("packages/dao-governance-client/src/index.ts"),
        },
        ContractDoc {
            heading: "Subscription Payments",
            crate_path: "crates/subscription-payments",
            package: "soroban-forge-subscription-payments",
            client_path: "packages/typescript-sdk/src/clients/subscription.ts",
            standalone_client: Some("packages/subscription-payments-client/src/index.ts"),
        },
        ContractDoc {
            heading: "Marketplace Royalties",
            crate_path: "crates/marketplace-royalties",
            package: "soroban-forge-marketplace-royalties",
            client_path: "packages/typescript-sdk/src/clients/marketplace.ts",
            standalone_client: Some("packages/marketplace-royalties-client/src/index.ts"),
        },
    ];

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
    }

    fn code_spans(text: &str) -> Vec<&str> {
        let mut spans = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('`') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find('`') else {
                break;
            };
            spans.push(&rest[..end]);
            rest = &rest[end + 1..];
        }
        spans
    }

    fn brace_body(source: &str, open: usize) -> &str {
        let mut depth = 0usize;
        for (offset, character) in source[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[open + 1..open + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unbalanced braces in source");
    }

    fn trait_methods(source: &str) -> Vec<&str> {
        let start = source
            .find("pub trait SorobanForge")
            .expect("contract trait must be present");
        let open = start + source[start..].find('{').expect("trait body must open");
        brace_body(source, open)
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("fn ")?;
                let name = rest.split('(').next()?;
                name.chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
                    .then_some(name)
            })
            .collect()
    }

    fn enum_variants<'a>(source: &'a str, name: &str) -> Vec<&'a str> {
        let marker = format!("pub enum {name} {{");
        let start = source.find(&marker).expect("error enum must be present");
        let open = start + marker.len() - 1;
        brace_body(source, open)
            .lines()
            .filter_map(|line| {
                let (name, value) = line.trim().split_once('=')?;
                value
                    .trim_start()
                    .chars()
                    .next()?
                    .is_ascii_digit()
                    .then_some(name.trim())
            })
            .collect()
    }

    fn client_methods(source: &str) -> Vec<&str> {
        let body = source
            .split_once("export interface Client {")
            .expect("client interface must be present")
            .1
            .split_once("\n}")
            .expect("client interface must close")
            .0;
        body.lines()
            .filter_map(|line| {
                if !line.starts_with("  ") || line.starts_with("   ") {
                    return None;
                }
                let (name, signature) = line.trim().split_once(':')?;
                let valid = signature.contains("Promise<")
                    && name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_');
                valid.then_some(name)
            })
            .collect()
    }

    fn matrix_section<'a>(matrix: &'a str, heading: &str) -> &'a str {
        let marker = format!("## {heading}\n");
        let start = matrix
            .find(&marker)
            .unwrap_or_else(|| panic!("missing matrix section for {heading}"));
        let rest = &matrix[start + marker.len()..];
        let end = rest.find("\n## ").unwrap_or(rest.len());
        &rest[..end]
    }

    fn matrix_subsection<'a>(section: &'a str, heading: &str) -> &'a str {
        let marker = format!("### {heading}\n");
        let start = section
            .find(&marker)
            .unwrap_or_else(|| panic!("missing subsection for {heading}"));
        let rest = &section[start + marker.len()..];
        let end = rest.find("\n### ").unwrap_or(rest.len());
        &rest[..end]
    }

    fn documented_methods(subsection: &str) -> Vec<&str> {
        subsection
            .lines()
            .filter_map(|line| {
                let (name, _) = line.strip_prefix("| `")?.split_once("` |")?;
                Some(name)
            })
            .collect()
    }

    fn row_cells(line: &str) -> Vec<&str> {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect()
    }

    fn sorted<'a>(values: &[&'a str]) -> Vec<&'a str> {
        let mut result = values.to_vec();
        result.sort_unstable();
        result
    }

    #[test]
    fn matrix_is_linked_from_documentation_indices() {
        let links = [
            (
                "docs/contracts/index.md",
                CONTRACTS_INDEX,
                "[Authorization Matrix](./authorization-matrix.md)",
            ),
            (
                "docs/contracts/README.md",
                CONTRACTS_README,
                "[Cross-Crate Authorization Matrix](./authorization-matrix.md)",
            ),
            (
                "docs/README.md",
                DOCS_INDEX,
                "[Authorization Matrix](contracts/authorization-matrix.md)",
            ),
        ];
        for (path, document, link) in links {
            assert!(
                document.contains(link),
                "{path} must link the matrix: {link}"
            );
        }
        assert!(workspace_root()
            .join("docs/contracts/authorization-matrix.md")
            .is_file());
    }

    #[test]
    fn documented_crate_and_client_paths_exist() {
        let root = workspace_root();
        for contract in CONTRACTS {
            let crate_dir = root.join(contract.crate_path);
            assert!(crate_dir.is_dir(), "{}", contract.crate_path);
            let manifest = crate_dir.join("Cargo.toml");
            assert!(manifest.is_file(), "{}", manifest.display());
            let manifest_text = fs::read_to_string(manifest).expect("manifest must be readable");
            let package_line = format!("name = \"{}\"", contract.package);
            assert!(
                manifest_text.contains(&package_line),
                "{} must declare {package_line}",
                contract.crate_path
            );
            assert!(
                root.join(contract.client_path).is_file(),
                "{}",
                contract.client_path
            );
            if let Some(path) = contract.standalone_client {
                assert!(root.join(path).is_file(), "{path}");
            }
            assert!(
                MATRIX.contains(contract.crate_path),
                "matrix must cite {}",
                contract.crate_path
            );
            assert!(
                MATRIX.contains(contract.package),
                "matrix must cite {}",
                contract.package
            );
            assert!(
                MATRIX.contains(contract.client_path),
                "matrix must cite {}",
                contract.client_path
            );
        }
        for path in code_spans(MATRIX) {
            if path.starts_with("crates/") || path.starts_with("packages/") {
                assert!(root.join(path).exists(), "broken matrix path: {path}");
            }
        }
    }

    #[test]
    fn matrix_entrypoints_match_traits_and_clients() {
        for contract in CONTRACTS {
            let source = fs::read_to_string(
                workspace_root()
                    .join(contract.crate_path)
                    .join("src/lib.rs"),
            )
            .expect("contract source must be readable");
            let expected = trait_methods(&source);
            let section = matrix_section(MATRIX, contract.heading);
            let mut documented =
                documented_methods(matrix_subsection(section, "Mutating entrypoints"));
            documented.extend(documented_methods(matrix_subsection(
                section,
                "Read-only queries",
            )));
            assert_eq!(
                sorted(&documented),
                sorted(&expected),
                "{} entrypoints differ from the contract trait",
                contract.heading
            );
            let client = fs::read_to_string(workspace_root().join(contract.client_path))
                .expect("client must be readable");
            assert_eq!(
                sorted(&documented),
                sorted(&client_methods(&client)),
                "{} entrypoints differ from the generated client",
                contract.heading
            );
        }
    }

    #[test]
    fn matrix_rows_are_complete() {
        let variants = enum_variants(ERRORS_SOURCE, "ForgeError");
        assert!(!variants.is_empty(), "ForgeError variants must be readable");
        for contract in CONTRACTS {
            let section = matrix_section(MATRIX, contract.heading);
            let mut names = Vec::new();
            for line in matrix_subsection(section, "Mutating entrypoints")
                .lines()
                .filter(|line| line.starts_with("| `"))
            {
                let cells = row_cells(line);
                assert_eq!(
                    cells.len(),
                    4,
                    "{} mutating row must have four cells: {line}",
                    contract.heading
                );
                for (index, cell) in cells.iter().enumerate() {
                    assert!(
                        !cell.is_empty(),
                        "{} mutating row has an empty cell {index}: {line}",
                        contract.heading
                    );
                }
                assert!(
                    code_spans(cells[2])
                        .iter()
                        .any(|code| variants.contains(code)),
                    "{} must name a known ForgeError variant: {line}",
                    contract.heading
                );
                names.push(cells[0]);
            }
            for line in matrix_subsection(section, "Read-only queries")
                .lines()
                .filter(|line| line.starts_with("| `"))
            {
                let cells = row_cells(line);
                assert_eq!(
                    cells.len(),
                    3,
                    "{} read-only row must have three cells: {line}",
                    contract.heading
                );
                for (index, cell) in cells.iter().enumerate() {
                    assert!(
                        !cell.is_empty(),
                        "{} read-only row has an empty cell {index}: {line}",
                        contract.heading
                    );
                }
                names.push(cells[0]);
            }
            let mut unique = names.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                names.len(),
                unique.len(),
                "{} matrix duplicates an entrypoint",
                contract.heading
            );
        }
    }
}
