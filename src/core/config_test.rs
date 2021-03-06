//
// Copyright (c) Pirmin Kalberer. All rights reserved.
// Licensed under the MIT License. See LICENSE file in the project root for full license information.
//

use core::config::read_config;
use core::config::ApplicationCfg;


#[test]
fn test_load_config() {
    let config = read_config("src/test/example.toml");
    println!("{:#?}", config);
    let config: ApplicationCfg = config.expect("load_config returned Err");
    assert!(config.service.mvt.viewer);
    assert_eq!(config.datasource.dstype, "postgis");
    assert_eq!(config.grid.predefined, Some("web_mercator".to_string()));
    assert_eq!(config.tilesets.len(), 1);
    assert_eq!(config.tilesets[0].name, "osm");
    assert_eq!(config.tilesets[0].layers.len(), 3);
    assert_eq!(config.tilesets[0].layers[0].name, "points");
    assert!(config.cache.is_none());
    assert_eq!(config.webserver.port, Some(8080));
}

#[test]
fn test_parse_error() {
    let config: Result<ApplicationCfg, _> = read_config("src/core/mod.rs");
    assert_eq!("src/core/mod.rs - unexpected character found: `/` at line 1",
               config.err().unwrap());

    let config: Result<ApplicationCfg, _> = read_config("wrongfile");
    assert_eq!("Could not find config file!", config.err().unwrap());
}
