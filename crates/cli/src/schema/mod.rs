use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StorageRegistry {
    pub generated_at: String,
    pub contracts: Vec<ContractSchema>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContractSchema {
    pub crate_name: String,
    pub contract_name: String,
    pub storage_kind: String,
    pub data_keys: Vec<DataKeyEntry>,
    pub records: Vec<StoredRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_enum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_variant_schema: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataKeyEntry {
    pub variant: String,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StoredRecord {
    pub name: String,
    pub storage_key: String,
    pub fields: Vec<(String, String)>,
}
