use serde_yaml;
use std::fs;
use std::io::{self};
use std::path::Path;
use std::process::Command;
use regex::Regex;

fn main() -> io::Result<()> {
    let mut path = String::new();
    let mut trimmed_path = String::new();
    
    if !conf_check() {
        println!("Config file not found or config missing! Attempting to create `/etc/opt/.yamlBackup.conf`");
        println!("Enter the dir path to store the backup:");
        io::stdin()
            .read_line(&mut path)
            .expect("Failed to read line");
        trimmed_path.push_str(path.trim());
        if trimmed_path.is_empty() {
            trimmed_path.push_str("/tmp/yamlBackup");
        }
        let cnf = format!("backupPath: {}", trimmed_path);
        fs::write("/etc/opt/.yamlBackup.conf", cnf)?;
        println!("Config created, backup path: {}", trimmed_path);
    } else {
        trimmed_path = get_path();
        println!("Config present, backing up yaml at: {}", trimmed_path);
    }

    let namespaces = get_namespaces()?;
    let kinds = [
        "deployment",
        "configmap",
        "ingress",
        "service",
        "secret",
        "pv",
        "pvc",
    ];
    
    for ns in namespaces {
        for kind in &kinds {
            let names = get_resource_names(kind, &ns)?;
            for name in names {
                let dir = format!("{}/{}/{}", trimmed_path, ns, kind);
                fs::create_dir_all(&dir)?;
                let yaml = get_resource_yaml(kind, &name, &ns)?;
                let cleaned_yaml = clean_yaml(&yaml)?;
                let file_path = format!("{}/{}.yml", dir, name);
                fs::write(&file_path, cleaned_yaml)?;
            }
            println!("YMLs of {}/{} resource backed up.", ns, kind);
        }
    }
    Ok(())
}

fn conf_check() -> bool {
    if Path::new("/etc/opt/.yamlBackup.conf").exists() {
        let data = fs::read_to_string("/etc/opt/.yamlBackup.conf").expect("File not found!");
        let regex = Regex::new(r"^backupPath: /.+").unwrap();
        match regex.is_match(&data) {
            true => true,
            _ => false,
        }
    } else {
        false
    }
}

fn get_path() -> String {
    let data = fs::read_to_string("/etc/opt/.yamlBackup.conf").expect("File not found!");
    let actual_path: Vec<&str> = data.split_whitespace().collect();
    actual_path[1].to_string()
}

fn get_namespaces() -> io::Result<Vec<String>> {
    let output = Command::new("kubectl")
        .args(&["get", "ns", "-o=custom-columns=:.metadata.name"])
        .output()?;

    let namespaces = String::from_utf8_lossy(&output.stdout);
    Ok(namespaces
        .lines()
        .filter(|line| {
            !line.is_empty() && !line.starts_with("kube-") && !line.starts_with("cattle-")
        })
        .map(String::from)
        .collect())
}

fn get_resource_names(kind: &str, namespace: &str) -> io::Result<Vec<String>> {
    let output = Command::new("kubectl")
        .args(&[
            "get",
            kind,
            "-n",
            namespace,
            "-o=jsonpath={.items[*].metadata.name}",
        ])
        .output()?;

    let names = String::from_utf8_lossy(&output.stdout);
    Ok(names.split_whitespace().map(String::from).collect())
}

fn get_resource_yaml(kind: &str, name: &str, namespace: &str) -> io::Result<String> {
    let output = Command::new("kubectl")
        .args(&[
            "get",
            &format!("{}/{}", kind, name),
            "-o",
            "yaml",
            "-n",
            namespace,
        ])
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn clean_yaml(yaml: &str) -> io::Result<String> {
    let mut yaml_value: serde_yaml::Value = serde_yaml::from_str(yaml)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if let serde_yaml::Value::Mapping(ref mut map) = yaml_value {
        if let Some(serde_yaml::Value::Mapping(ref mut metadata)) = map.get_mut(&serde_yaml::Value::String("metadata".to_string())) {
            if let Some(serde_yaml::Value::Mapping(ref mut annotations)) = metadata.get_mut(&serde_yaml::Value::String("annotations".to_string())) {
                let keys_to_remove = [
                    "field.cattle.io/targetWorkloadIds",
                    "workload.cattle.io/targetWorkloadIdNoop",
                    "workload.cattle.io/workloadPortBased",
                    "field.cattle.io/ingressState",
                    "field.cattle.io/publicEndpoints",
                    "deployment.kubernetes.io/revision",
                    "field.cattle.io/creatorId",
                    "cattle.io/timestamp",
                    "kubectl.kubernetes.io/last-applied-configuration",
                    "objectset.rio.cattle.io/id",
                    "objectset.rio.cattle.io/applied"
                ];

                for key in &keys_to_remove {
                    annotations.remove(&serde_yaml::Value::String(key.to_string()));
                }

                if annotations.is_empty() {
                    metadata.remove(&serde_yaml::Value::String("annotations".to_string()));
                }
            }

            let metadata_fields_to_remove = [
                "managedFields",
                "creationTimestamp",
                "resourceVersion",
                "selfLink",
                "uid",
                "generation",
                "ownerReferences"
            ];

            for field in &metadata_fields_to_remove {
                metadata.remove(&serde_yaml::Value::String(field.to_string()));
            }

            if let Some(serde_yaml::Value::Mapping(ref mut labels)) = metadata.get_mut(&serde_yaml::Value::String("labels".to_string())) {
                labels.remove(&serde_yaml::Value::String("cattle.io/creator".to_string()));
                
                if labels.is_empty() {
                    metadata.remove(&serde_yaml::Value::String("labels".to_string()));
                }
            }
        }

        map.remove(&serde_yaml::Value::String("status".to_string()));

        if let Some(serde_yaml::Value::Mapping(ref mut spec)) = map.get_mut(&serde_yaml::Value::String("spec".to_string())) {
            if let Some(serde_yaml::Value::Mapping(ref mut template)) = spec.get_mut(&serde_yaml::Value::String("template".to_string())) {
                if let Some(serde_yaml::Value::Mapping(ref mut template_metadata)) = template.get_mut(&serde_yaml::Value::String("metadata".to_string())) {
                    template_metadata.remove(&serde_yaml::Value::String("annotations".to_string()));
                    template_metadata.remove(&serde_yaml::Value::String("creationTimestamp".to_string()));
                }
            }
        }
    }

    serde_yaml::to_string(&yaml_value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
