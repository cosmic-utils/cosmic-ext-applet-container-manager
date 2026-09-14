use cosmic_ext_applet_container_manager::{domain::Backend, parser::parse_container_list};

#[test]
fn parses_runtime_fields_and_reports_malformed_rows() {
    let row = "0123456789abcdef\tregistry/team/web:latest\tmy-web\trunning\tUp 3 hours\n";
    let items = parse_container_list(Backend::Podman, row).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].backend, Backend::Podman);
    assert_eq!(items[0].name, "my-web");
    assert_eq!(items[0].status, "Up 3 hours");
    let error = parse_container_list(Backend::Docker, "id\ttoo-few\n").unwrap_err();
    assert!(error.to_string().contains("line 1"));
}
