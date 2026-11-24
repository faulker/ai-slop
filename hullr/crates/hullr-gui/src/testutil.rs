//! Fixtures shared by unit tests.

use hullr_core::*;

pub fn platform() -> Platform {
    Platform::new("linux", "arm64", Some("v8"))
}

pub fn container(name: &str, state: ContainerState) -> ContainerInfo {
    ContainerInfo {
        id: format!("{:0<64}", format!("{:x}", name.bytes().map(u64::from).sum::<u64>() * 7919)),
        name: name.into(),
        image: "nginx:latest".into(),
        image_id: "sha256:abc".into(),
        command: vec!["nginx".into()],
        created: 1_700_000_000,
        state,
        exit_code: if state == ContainerState::Exited { Some(0) } else { None },
        pid: None,
        started_at: Some(1_700_000_000),
        finished_at: Some(1_700_000_100),
        health: None,
        platform: platform(),
        spec: ContainerSpec {
            image: "nginx:latest".into(),
            ports: vec![PortMapping::parse("8080:80").unwrap()],
            ..Default::default()
        },
    }
}

pub fn image(tag: &str) -> ImageInfo {
    ImageInfo {
        id: format!("sha256:{:0<64}", tag.len()),
        repo_tags: vec![tag.into()],
        size: 142_000_000,
        created: 1_700_000_000,
        platform: platform(),
        config: ImageConfig::default(),
    }
}

pub fn volume(name: &str) -> VolumeInfo {
    VolumeInfo { name: name.into(), path: format!("/data/{name}").into(), created: 1_700_000_000, labels: Default::default() }
}
