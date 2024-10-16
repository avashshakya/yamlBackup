use regex::Regex;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

fn main() -> io::Result<()> {
    // Check if yq is installed
    if Command::new("yq").output().is_err() {
        println!("yq not found, attempting to download.");
        let url = "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64";
        let output_path = "/usr/bin/yq";
        if download_yq(url, output_path).is_ok() {
            let _ = Command::new("chmod").arg("+x").arg(output_path).output()?;
        } else {
            eprintln!("ERROR: Unable to get yq");
            std::process::exit(1);
        }
    }
    let mut path = String::new();
    let mut trimmed_path = String::new();
    // check if the config file exists
    if !conf_check() {
        // get the location for storing the backup yaml
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

    // Get namespaces
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
                // println!("dir: {}", dir);

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

fn download_yq(url: &str, output_path: &str) -> io::Result<()> {
    let output = Command::new("curl")
        .arg("-L")
        .arg(url)
        .arg("-o")
        .arg(output_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "Download failed"));
    }
    Ok(())
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
    let mut child = Command::new("yq")
        .arg("eval")
        .arg(
            r#"del(.metadata.annotations["field.cattle.io/targetWorkloadIds"],
                     .metadata.annotations["workload.cattle.io/targetWorkloadIdNoop"],
                     .metadata.annotations["workload.cattle.io/workloadPortBased"],
                     .metadata.annotations["field.cattle.io/ingressState"],
                     .metadata.annotations["field.cattle.io/publicEndpoints"],
                     .metadata.annotations["deployment.kubernetes.io/revision"],
                     .metadata.annotations["field.cattle.io/creatorId"],
                     .metadata.annotations["cattle.io/timestamp"],
                     .metadata.annotations["kubectl.kubernetes.io/last-applied-configuration"],
                     .metadata.annotations["objectset.rio.cattle.io/id"],
                     .metadata.annotations["objectset.rio.cattle.io/applied"],
                     .metadata.managedFields,
                     .metadata.creationTimestamp,
                     .metadata.resourceVersion,
                     .metadata.selfLink,
                     .metadata.uid,
                     .metadata.labels["cattle.io/creator"],
                     .metadata.generation,
                     .metadata.ownerReferences,
                     .status,
                     .spec.template.metadata.annotations,
                     .spec.template.metadata.creationTimestamp)
            | del(.metadata.annotations | select(. != null and length == 0))
            | del(.metadata.labels | select(. != null and length == 0))"#,
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?; // Spawn the command

    {
        let stdin = child.stdin.as_mut().expect("Failed to open stdin");
        stdin.write_all(yaml.as_bytes())?;
    } // Drop stdin to signal EOF

    let output = child.wait_with_output()?; // Wait for the command to finish

    if !output.status.success() {
        return Err(io::Error::new(io::ErrorKind::Other, "yq command failed"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

