//! Compose loader tests with inline YAML fixtures.

use super::*;
use hullr_core::{MountKind, Protocol};

struct Fx {
    _td: tempfile::TempDir,
    dir: PathBuf,
}

fn fx(files: &[(&str, &str)]) -> Fx {
    let td = tempfile::tempdir().unwrap();
    let dir = td.path().join("My_Proj");
    std::fs::create_dir_all(&dir).unwrap();
    for (n, c) in files {
        std::fs::write(dir.join(n), c).unwrap();
    }
    Fx { _td: td, dir }
}

fn opts(f: &Fx) -> ComposeOptions {
    ComposeOptions { project_dir: f.dir.clone(), ..Default::default() }
}

fn load_one(yaml: &str) -> Result<Project> {
    let f = fx(&[("compose.yaml", yaml)]);
    load_with_process_env(&opts(&f), &BTreeMap::new())
}

fn ok(yaml: &str) -> Project {
    load_one(yaml).unwrap()
}

#[test]
fn project_name_precedence() {
    let f = fx(&[("compose.yaml", "name: Filename\nservices: {a: {image: x}}")]);
    let mut o = opts(&f);
    let mut env = BTreeMap::new();
    env.insert("COMPOSE_PROJECT_NAME".to_string(), "envname".to_string());
    assert_eq!(load_with_process_env(&o, &env).unwrap().name, "filename");
    o.project_name = Some("CLI Name!".into());
    assert_eq!(load_with_process_env(&o, &env).unwrap().name, "cliname");

    let f = fx(&[("compose.yaml", "services: {a: {image: x}}")]);
    assert_eq!(load_with_process_env(&opts(&f), &env).unwrap().name, "envname");
    assert_eq!(load_with_process_env(&opts(&f), &BTreeMap::new()).unwrap().name, "my_proj");
    assert_eq!(normalize_project_name("..."), "default");
    assert_eq!(normalize_project_name("_Foo-Bar.9"), "foo-bar9");
}

#[test]
fn file_discovery_and_override() {
    let f = fx(&[
        ("docker-compose.yml", "services: {a: {image: base, restart: always}}"),
        ("docker-compose.override.yml", "services: {a: {image: over}, b: {image: b}}"),
    ]);
    let p = load_with_process_env(&opts(&f), &BTreeMap::new()).unwrap();
    assert_eq!(p.services["a"].image.as_deref(), Some("over"));
    assert_eq!(p.services["a"].restart, RestartPolicy::Always);
    assert_eq!(p.services.keys().collect::<Vec<_>>(), ["a", "b"]);

    let f = fx(&[]);
    assert!(matches!(load_with_process_env(&opts(&f), &BTreeMap::new()), Err(Error::NotFound(_))));
}

#[test]
fn explicit_files_merge_in_order() {
    let f = fx(&[
        ("a.yaml", "services: {s: {image: x, ports: ['80:80'], command: [a], environment: {A: '1'}}}"),
        ("b.yaml", "services: {s: {ports: ['80:80', '81:81'], command: 'b c', environment: [A=2, B=3]}}"),
    ]);
    let mut o = opts(&f);
    o.files = vec!["a.yaml".into(), f.dir.join("b.yaml")];
    let p = load_with_process_env(&o, &BTreeMap::new()).unwrap();
    let s = &p.services["s"];
    assert_eq!(s.ports.len(), 2);
    assert_eq!(s.command.as_deref(), Some(&["b".to_string(), "c".to_string()][..]));
    assert_eq!(s.environment, ["A=2", "B=3"]);
}

#[test]
fn interpolation_precedence_and_forms() {
    let f = fx(&[
        (".env", "FROM_DOTENV=dot\nSHARED=dot\nTAG=1.0\n"),
        (
            "compose.yaml",
            "services:\n  a:\n    image: \"app:${TAG}\"\n    command: echo $$HOME ${MISSING:-dflt} ${SHARED}\n    environment:\n      X: ${FROM_DOTENV}\n      Y: ${OPT_ONLY:-nope}\n",
        ),
    ]);
    let mut o = opts(&f);
    o.env.insert("TAG".into(), "2.0".into());
    let mut proc = BTreeMap::new();
    proc.insert("SHARED".to_string(), "proc".to_string());
    proc.insert("TAG".to_string(), "proc-tag".to_string());
    let p = load_with_process_env(&o, &proc).unwrap();
    let s = &p.services["a"];
    assert_eq!(s.image.as_deref(), Some("app:2.0"));
    assert_eq!(s.command.as_ref().unwrap(), &["echo", "$HOME", "dflt", "proc"]);
    assert_eq!(s.environment, ["X=dot", "Y=nope"]);
}

#[test]
fn interpolation_required_error() {
    let e = load_one("services: {a: {image: '${NEED:?set NEED}'}}").unwrap_err();
    assert!(e.to_string().contains("set NEED"), "{e}");
}

#[test]
fn ports_short_long_and_ranges() {
    let p = ok(r#"
services:
  a:
    image: x
    ports:
      - "8080:80"
      - "127.0.0.1:53:53/udp"
      - 3000
      - "8000-8002:9000-9002"
      - target: 443
        published: 8443
        protocol: tcp
        host_ip: 10.0.0.1
      - target: 22
"#);
    let ports = &p.services["a"].ports;
    assert_eq!(ports.len(), 1 + 1 + 1 + 3 + 1 + 1);
    assert_eq!((ports[0].host_port, ports[0].container_port), (8080, 80));
    assert_eq!(ports[1].protocol, Protocol::Udp);
    assert_eq!(ports[1].host_ip.as_deref(), Some("127.0.0.1"));
    assert_eq!((ports[2].host_port, ports[2].container_port), (3000, 3000));
    assert_eq!((ports[3].host_port, ports[3].container_port), (8000, 9000));
    assert_eq!((ports[5].host_port, ports[5].container_port), (8002, 9002));
    assert_eq!(ports[6].host_ip.as_deref(), Some("10.0.0.1"));
    assert_eq!((ports[6].host_port, ports[6].container_port), (8443, 443));
    assert_eq!((ports[7].host_port, ports[7].container_port), (22, 22));
    assert_eq!(p.all_ports().len(), 8);
}

#[test]
fn port_errors() {
    assert!(load_one("services: {a: {image: x, ports: ['8000-8002:80-81']}}").is_err());
    assert!(load_one("services: {a: {image: x, ports: ['x:80']}}").is_err());
    assert!(load_one("services: {a: {image: x, ports: ['80/sctp']}}").is_err());
}

#[test]
fn volumes_short_long_named_external() {
    let p = ok(r#"
name: demo
services:
  a:
    image: x
    volumes:
      - ./data:/data:ro
      - /abs:/abs
      - pgdata:/var/lib/pg
      - ext:/ext
      - named:/named
      - /anon
      - type: bind
        source: ../up
        target: /up
        read_only: true
      - type: volume
        source: pgdata
        target: /other
      - type: tmpfs
        target: /tmp
    tmpfs: /run
volumes:
  pgdata:
  ext:
    external: true
  named:
    name: custom_name
"#);
    let v = &p.services["a"].volumes;
    let dir = p.dir.to_string_lossy().into_owned();
    assert_eq!(v[0].kind, MountKind::Bind);
    assert_eq!(v[0].source, format!("{dir}/data"));
    assert!(v[0].read_only);
    assert_eq!(v[1].source, "/abs");
    assert_eq!((v[2].kind, v[2].source.as_str()), (MountKind::Volume, "demo_pgdata"));
    assert_eq!(v[3].source, "ext");
    assert_eq!(v[4].source, "custom_name");
    assert_eq!((v[5].kind, v[5].source.as_str()), (MountKind::Volume, ""));
    assert_eq!(v[6].kind, MountKind::Bind);
    assert!(v[6].read_only);
    assert!(v[6].source.ends_with("/up") && !v[6].source.contains(".."));
    assert_eq!(v[7].source, "demo_pgdata");
    assert_eq!(v[8].kind, MountKind::Tmpfs);
    assert_eq!((v[9].kind, v[9].target.as_str()), (MountKind::Tmpfs, "/run"));
    assert!(p.volumes["ext"].external);
}

#[test]
fn undeclared_volume_warns() {
    let p = ok("name: d\nservices: {a: {image: x, volumes: ['v:/v']}}");
    assert_eq!(p.services["a"].volumes[0].source, "d_v");
    assert!(p.warnings.iter().any(|w| w.contains("'v'")));
}

#[test]
fn environment_and_env_file() {
    let f = fx(&[
        ("base.env", "A=file\nB=file\n# c\nC=\"quoted v\"\n"),
        ("other.env", "D=${HOSTV}\n"),
        (
            "compose.yaml",
            r#"
services:
  a:
    image: x
    env_file:
      - base.env
      - path: other.env
      - path: missing.env
        required: false
    environment:
      B: override
      HOSTVAR:
      NOTSET:
      NUM: 5
"#,
        ),
    ]);
    let mut proc = BTreeMap::new();
    proc.insert("HOSTVAR".to_string(), "from-host".to_string());
    proc.insert("HOSTV".to_string(), "hv".to_string());
    let p = load_with_process_env(&opts(&f), &proc).unwrap();
    assert_eq!(
        p.services["a"].environment,
        ["A=file", "B=override", "C=quoted v", "D=hv", "HOSTVAR=from-host", "NUM=5"]
    );

    let f = fx(&[("compose.yaml", "services: {a: {image: x, env_file: nope.env}}")]);
    assert!(load_with_process_env(&opts(&f), &BTreeMap::new()).is_err());
    let f = fx(&[("compose.yaml", "services: {a: {image: x, env_file: [e.env], environment: [K, L=2]}}"), ("e.env", "K=1")]);
    let p = load_with_process_env(&opts(&f), &BTreeMap::new()).unwrap();
    assert_eq!(p.services["a"].environment, ["K=1", "L=2"]);
}

#[test]
fn depends_on_forms_and_errors() {
    let p = ok(r#"
services:
  a: {image: x, depends_on: [b]}
  b:
    image: x
    depends_on:
      c: {condition: service_healthy}
      d: {condition: service_completed_successfully}
  c: {image: x}
  d: {image: x}
"#);
    assert_eq!(p.services["a"].depends_on[0].condition, DependsCondition::Started);
    assert_eq!(p.services["b"].depends_on[0].condition, DependsCondition::Healthy);
    assert_eq!(p.services["b"].depends_on[1].condition, DependsCondition::CompletedSuccessfully);

    let e = load_one("services: {a: {image: x, depends_on: [ghost]}}").unwrap_err();
    assert!(e.to_string().contains("ghost"));
    assert!(load_one("services: {a: {image: x, depends_on: {a: {condition: weird}}}}").is_err());
}

#[test]
fn command_and_entrypoint() {
    let p = ok(r#"
services:
  a:
    image: x
    entrypoint: /bin/app --flag "two words" 'x y'
    command: ["--port", "80"]
  b:
    image: x
    command: sh -c "echo hi && sleep 1"
    entrypoint: []
"#);
    assert_eq!(p.services["a"].entrypoint.as_ref().unwrap(), &["/bin/app", "--flag", "two words", "x y"]);
    assert_eq!(p.services["a"].command.as_ref().unwrap(), &["--port", "80"]);
    assert_eq!(p.services["b"].command.as_ref().unwrap(), &["sh", "-c", "echo hi && sleep 1"]);
    assert_eq!(p.services["b"].entrypoint, Some(vec![]));
}

#[test]
fn healthcheck_forms() {
    let p = ok(r#"
services:
  a:
    image: x
    healthcheck:
      test: pg_isready -U me
      interval: 1m30s
      timeout: 500ms
      retries: 5
      start_period: 10s
  b:
    image: x
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost"]
  c:
    image: x
    healthcheck:
      disable: true
"#);
    let a = p.services["a"].healthcheck.as_ref().unwrap();
    assert_eq!(a.test, ["CMD-SHELL", "pg_isready -U me"]);
    assert_eq!((a.interval_secs, a.timeout_secs, a.retries, a.start_period_secs), (90, 1, 5, 10));
    let b = p.services["b"].healthcheck.as_ref().unwrap();
    assert_eq!(b.test[0], "CMD");
    assert_eq!(b.interval_secs, 30);
    assert_eq!(p.services["c"].healthcheck.as_ref().unwrap().test, ["NONE"]);
    assert!(load_one("services: {a: {image: x, healthcheck: {test: [curl]}}}").is_err());
    assert!(load_one("services: {a: {image: x, healthcheck: {interval: 1s}}}").is_err());
}

#[test]
fn build_forms() {
    let p = ok(r#"
services:
  a:
    build: ./app
  b:
    build:
      context: ./b
      dockerfile: Dockerfile.dev
      target: dev
      args:
        X: "1"
        FROMENV:
  c:
    build:
      context: .
      args: [K=v]
    image: custom:tag
"#);
    let d = p.dir.to_string_lossy().into_owned();
    let a = p.services["a"].build.as_ref().unwrap();
    assert_eq!(a.context, PathBuf::from(format!("{d}/app")));
    let b = p.services["b"].build.as_ref().unwrap();
    assert_eq!(b.dockerfile.as_ref().unwrap(), &PathBuf::from(format!("{d}/b/Dockerfile.dev")));
    assert_eq!(b.target.as_deref(), Some("dev"));
    assert_eq!(b.args.len(), 1);
    assert_eq!(p.services["c"].build.as_ref().unwrap().args["K"], "v");
    assert_eq!(p.image_name("a"), "my_proj-a");
    assert_eq!(p.image_name("c"), "custom:tag");
}

#[test]
fn misc_fields() {
    let p = ok(r#"
services:
  a:
    image: x
    restart: on-failure:3
    labels: [a=b, flag]
    extra_hosts: ["db.local:10.0.0.5"]
    mem_limit: 512m
    cpus: 1.5
    platform: linux/arm64
    tty: true
    stdin_open: "true"
    hostname: box
    user: "1000:1000"
    working_dir: /srv
    container_name: fixed
  b:
    image: x
    labels: {k: v}
    extra_hosts: {h: 1.2.3.4}
    mem_limit: 1g
"#);
    let a = &p.services["a"];
    assert_eq!(a.restart, RestartPolicy::OnFailure(3));
    assert_eq!(a.labels["a"], "b");
    assert_eq!(a.labels["flag"], "");
    assert_eq!(a.extra_hosts, [("db.local".to_string(), "10.0.0.5".to_string())]);
    assert_eq!(a.mem_limit, Some(512 << 20));
    assert_eq!(a.cpus, Some(1.5));
    assert!(a.tty && a.stdin_open);
    assert_eq!(p.container_name("a"), "fixed");
    assert_eq!(p.container_name("b"), "my_proj-b-1");
    assert_eq!(p.services["b"].extra_hosts[0].0, "h");
    assert_eq!(p.services["b"].mem_limit, Some(1 << 30));
}

#[test]
fn deploy_limits_fallback() {
    let p = ok("services: {a: {image: x, deploy: {resources: {limits: {memory: 256M, cpus: '0.5'}}}}}");
    assert_eq!(p.services["a"].mem_limit, Some(256 << 20));
    assert_eq!(p.services["a"].cpus, Some(0.5));
}

#[test]
fn errors_and_warnings() {
    let e = load_one("services: {a: {command: x}}").unwrap_err();
    assert!(e.to_string().contains("image") && e.to_string().contains("'a'"));
    let p = ok("x-ext: {}\nsecrets: {}\nservices: {a: {image: x, cap_add: [ALL], x-foo: 1, networks: [n]}}");
    assert!(p.warnings.iter().any(|w| w.contains("cap_add")));
    assert!(p.warnings.iter().any(|w| w.contains("secrets")));
    assert!(!p.warnings.iter().any(|w| w.contains("x-") || w.contains("networks")));
    assert!(load_one("nothing: 1").is_err());
}

#[test]
fn yaml_anchors_and_merge_keys() {
    let p = ok(r#"
x-common: &common
  restart: always
  environment: {A: "1"}
services:
  a:
    <<: *common
    image: x
"#);
    assert_eq!(p.services["a"].restart, RestartPolicy::Always);
    assert_eq!(p.services["a"].environment, ["A=1"]);
}

#[test]
fn profiles() {
    let yaml = "services: {a: {image: x}, dbg: {image: x, profiles: [debug]}, t: {image: x, profiles: [test, ci]}}";
    let p = ok(yaml);
    assert_eq!(p.services.keys().collect::<Vec<_>>(), ["a"]);

    let f = fx(&[("compose.yaml", yaml)]);
    let mut o = opts(&f);
    o.env.insert("COMPOSE_PROFILES".into(), "ci, debug".into());
    let p = load_with_process_env(&o, &BTreeMap::new()).unwrap();
    assert_eq!(p.services.len(), 3);

    let mut o = opts(&f);
    o.env.insert("COMPOSE_PROFILES".into(), "*".into());
    assert_eq!(load_with_process_env(&o, &BTreeMap::new()).unwrap().services.len(), 3);

    // explicitly requested services are enabled
    let mut o = opts(&f);
    o.services = vec!["dbg".into()];
    assert!(load_with_process_env(&o, &BTreeMap::new()).unwrap().services.contains_key("dbg"));

    let e = load_one("services: {a: {image: x, depends_on: [d]}, d: {image: x, profiles: [p]}}").unwrap_err();
    assert!(e.to_string().contains("profiles"));
}

fn order_fixture() -> Project {
    ok(r#"
services:
  web: {image: x, depends_on: [api, cache]}
  api: {image: x, depends_on: [db]}
  cache: {image: x}
  db: {image: x}
  other: {image: x}
"#)
}

#[test]
fn start_order_topological_stable() {
    let p = order_fixture();
    assert_eq!(p.start_order(&[]).unwrap(), ["cache", "db", "api", "web", "other"]);
    assert_eq!(p.start_order(&["api".into()]).unwrap(), ["db", "api"]);
    assert_eq!(p.start_order(&["web".into(), "other".into()]).unwrap(), ["cache", "db", "api", "web", "other"]);
    assert!(matches!(p.start_order(&["zzz".into()]), Err(Error::NotFound(_))));
}

#[test]
fn start_order_cycle() {
    let p = ok("services: {a: {image: x, depends_on: [b]}, b: {image: x, depends_on: [c]}, c: {image: x, depends_on: [b]}, d: {image: x}}");
    let e = p.start_order(&[]).unwrap_err();
    match e {
        Error::Invalid(m) => assert!(m.contains("b -> c -> b"), "{m}"),
        o => panic!("{o:?}"),
    }
    // d alone is fine
    assert_eq!(p.start_order(&["d".into()]).unwrap(), ["d"]);
    let p = ok("services: {a: {image: x, depends_on: [a]}}");
    assert!(p.start_order(&[]).unwrap_err().to_string().contains("a -> a"));
}

#[test]
fn realistic_project() {
    let f = fx(&[
        (".env", "POSTGRES_PASSWORD=s3cret\nWEB_PORT=8080\n"),
        (
            "compose.yaml",
            r#"
services:
  web:
    build:
      context: ./web
      args:
        APP_ENV: production
    ports:
      - "${WEB_PORT:-80}:3000"
    environment:
      DATABASE_URL: postgres://app:${POSTGRES_PASSWORD}@db:5432/app
      REDIS_URL: redis://redis:6379
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_started
    volumes:
      - ./web/uploads:/app/uploads
    restart: unless-stopped
  db:
    image: postgres:16
    environment:
      POSTGRES_PASSWORD:
      POSTGRES_USER: app
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U app"]
      interval: 5s
      timeout: 3s
      retries: 10
  redis:
    image: redis:7-alpine
    volumes:
      - redisdata:/data
volumes:
  pgdata:
  redisdata:
"#,
        ),
    ]);
    let p = load_with_process_env(&opts(&f), &BTreeMap::new()).unwrap();
    assert_eq!(p.name, "my_proj");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.start_order(&[]).unwrap(), ["db", "redis", "web"]);
    assert_eq!(p.start_order(&["web".into()]).unwrap(), ["db", "redis", "web"]);
    assert_eq!(p.services["web"].ports[0].host_port, 8080);
    assert_eq!(p.services["db"].environment, ["POSTGRES_PASSWORD=s3cret", "POSTGRES_USER=app"]);

    let spec = p.container_spec("web").unwrap();
    assert_eq!(spec.image, "my_proj-web");
    assert_eq!(spec.name.as_deref(), Some("my_proj-web-1"));
    assert_eq!(spec.network, NetworkMode::Sandbox("my_proj".into()));
    assert_eq!(spec.hostname.as_deref(), Some("web"));
    assert_eq!(spec.labels[labels::PROJECT], "my_proj");
    assert_eq!(spec.labels[labels::SERVICE], "web");
    assert_eq!(spec.labels[labels::CONFIG_HASH].len(), 16);
    assert_eq!(spec.restart, RestartPolicy::UnlessStopped);
    for h in ["web", "db", "redis"] {
        assert!(spec.extra_hosts.contains(&(h.to_string(), "127.0.0.1".to_string())));
    }
    let db = p.container_spec("db").unwrap();
    assert!(db.healthcheck.is_some());
    assert_eq!(db.mounts[0].source, "my_proj_pgdata");
    assert!(p.container_spec("nope").is_err());
}

#[test]
fn container_spec_fields() {
    let p = ok(r#"
services:
  a:
    image: x
    container_name: fixed
    platform: linux/amd64
    tty: true
    stdin_open: true
    user: bob
    working_dir: /w
    hostname: h
    mem_limit: 64m
    cpus: 2
    extra_hosts: ["x.test:9.9.9.9"]
    command: [run]
    entrypoint: [/e]
  b: {image: y}
"#);
    let s = p.container_spec("a").unwrap();
    assert_eq!(s.name.as_deref(), Some("fixed"));
    assert_eq!(s.platform.as_ref().unwrap().to_string(), "linux/amd64");
    assert!(s.tty && s.interactive);
    assert_eq!((s.user.as_deref(), s.working_dir.as_deref(), s.hostname.as_deref()), (Some("bob"), Some("/w"), Some("h")));
    assert_eq!(s.memory_limit, Some(64 << 20));
    assert_eq!(s.cpus, Some(2.0));
    assert_eq!(s.cmd.as_ref().unwrap(), &["run"]);
    assert_eq!(s.entrypoint.as_ref().unwrap(), &["/e"]);
    assert!(s.extra_hosts.contains(&("fixed".to_string(), "127.0.0.1".to_string())));
    assert!(s.extra_hosts.contains(&("x.test".to_string(), "9.9.9.9".to_string())));
    assert!(s.extra_hosts.contains(&("b".to_string(), "127.0.0.1".to_string())));
}

#[test]
fn config_hash_is_stable_and_sensitive() {
    let a = ok("services: {s: {image: x, environment: {A: '1'}}}");
    let a2 = ok("services: {s: {image: x, environment: {A: '1'}}}");
    let b = ok("services: {s: {image: x, environment: {A: '2'}}}");
    assert_eq!(a.config_hash("s"), a2.config_hash("s"));
    assert_ne!(a.config_hash("s"), b.config_hash("s"));
    assert_eq!(a.config_hash("s").len(), 16);
}
