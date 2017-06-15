//
// Copyright (c) Pirmin Kalberer. All rights reserved.
// Licensed under the MIT License. See LICENSE file in the project root for full license information.
//

use datasource::DatasourceInput;
use postgres::rows::Row;
use postgres::types::{Type, FromSql, ToSql};
use postgres;
use r2d2;
use r2d2_postgres::{PostgresConnectionManager, TlsMode};
use std;
use core::feature::{Feature, FeatureAttr, FeatureAttrValType};
use core::geom::*;
use core::grid::Extent;
use core::grid::Grid;
use core::layer::Layer;
use core::Config;
use toml;
use std::collections::BTreeMap;
use env;


impl GeometryType {
    pub fn from_geom_field(row: &Row, idx: &str, type_name: &str) -> Result<GeometryType, String> {
        let field = match type_name {
            //Option<Result<T>> --> Option<Result<GeometryType>>
            "POINT" => {
                row.get_opt::<_, Point>(idx)
                    .map(|opt| opt.map(|f| GeometryType::Point(f)))
            }
            //"LINESTRING" =>
            //    row.get_opt::<_, LineString>(idx).map(|opt| opt.map(|f| GeometryType::LineString(f))),
            //"POLYGON" =>
            //    row.get_opt::<_, Polygon>(idx).map(|opt| opt.map(|f| GeometryType::Polygon(f))),
            "MULTIPOINT" => {
                row.get_opt::<_, MultiPoint>(idx)
                    .map(|opt| opt.map(|f| GeometryType::MultiPoint(f)))
            }
            "LINESTRING" |
            "MULTILINESTRING" => {
                row.get_opt::<_, MultiLineString>(idx)
                    .map(|opt| opt.map(|f| GeometryType::MultiLineString(f)))
            }
            "POLYGON" | "MULTIPOLYGON" => {
                row.get_opt::<_, MultiPolygon>(idx)
                    .map(|opt| opt.map(|f| GeometryType::MultiPolygon(f)))
            }
            "GEOMETRYCOLLECTION" => {
                row.get_opt::<_, GeometryCollection>(idx)
                    .map(|opt| opt.map(|f| GeometryType::GeometryCollection(f)))
            }
            _ => {
                let err: Box<std::error::Error + Sync + Send> =
                    format!("Unknown geometry type {}", type_name).into();
                Some(Err(postgres::error::Error::Conversion(err)))
            }
        };
        // Option<Result<GeometryType, _>> --> Result<GeometryType, String>
        field.map_or_else(|| Err("Column not found".to_string()),
                          |res| res.map_err(|err| format!("{}", err)))
    }
}

// http://sfackler.github.io/rust-postgres/doc/v0.11.8/postgres/types/trait.FromSql.html#types
// http://sfackler.github.io/rust-postgres/doc/v0.11.8/postgres/types/enum.Type.html
impl FromSql for FeatureAttrValType {
    fn accepts(ty: &Type) -> bool {
        match ty {
            &Type::Varchar | &Type::Text | &Type::CharArray | &Type::Float4 | &Type::Float8 |
            &Type::Int2 | &Type::Int4 | &Type::Int8 | &Type::Bool => true,
            _ => false,
        }
    }
    fn from_sql(ty: &Type, raw: &[u8]) -> Result<Self, Box<std::error::Error + Sync + Send>> {
        match ty {
            &Type::Varchar | &Type::Text | &Type::CharArray => {
                <String>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::String(v)))
            }
            &Type::Float4 => {
                <f32>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Float(v)))
            }
            &Type::Float8 => {
                <f64>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Double(v)))
            }
            &Type::Int2 => {
                <i16>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Int(v as i64)))
            }
            &Type::Int4 => {
                <i32>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Int(v as i64)))
            }
            &Type::Int8 => <i64>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Int(v))),
            &Type::Bool => <bool>::from_sql(ty, raw).and_then(|v| Ok(FeatureAttrValType::Bool(v))),
            _ => {
                let err: Box<std::error::Error + Sync + Send> =
                    format!("cannot convert {} to FeatureAttrValType", ty).into();
                Err(err)
            }
        }
    }
}

struct FeatureRow<'a> {
    layer: &'a Layer,
    row: &'a Row<'a>,
}

impl<'a> Feature for FeatureRow<'a> {
    fn fid(&self) -> Option<u64> {
        self.layer
            .fid_field
            .as_ref()
            .and_then(|fid| {
                          let val = self.row.get_opt::<_, FeatureAttrValType>(fid as &str);
                          match val {
                              Some(Ok(FeatureAttrValType::Int(fid))) => Some(fid as u64),
                              _ => None,
                          }
                      })
    }
    fn attributes(&self) -> Vec<FeatureAttr> {
        let mut attrs = Vec::new();
        for (i, col) in self.row.columns().into_iter().enumerate() {
            if col.name() !=
               self.layer
                   .geometry_field
                   .as_ref()
                   .unwrap_or(&"".to_string()) {
                let val = self.row.get_opt::<_, Option<FeatureAttrValType>>(i);
                match val.unwrap() {
                    Ok(Some(v)) => {
                        let fattr = FeatureAttr {
                            key: col.name().to_string(),
                            value: v,
                        };
                        attrs.push(fattr);
                    }
                    Ok(None) => {
                        // Skip NULL values
                    }
                    Err(err) => {
                        warn!("Layer '{}' - skipping field '{}': {}",
                              self.layer.name,
                              col.name(),
                              err);
                        //warn!("{:?}", self.row);
                    }
                }
            }
        }
        attrs
    }
    fn geometry(&self) -> Result<GeometryType, String> {
        let geom = GeometryType::from_geom_field(&self.row,
                                                 &self.layer.geometry_field.as_ref().unwrap(),
                                                 &self.layer.geometry_type.as_ref().unwrap());
        if let Err(ref err) = geom {
            error!("Layer '{}': {}", self.layer.name, err);
            error!("{:?}", self.row);
        }
        geom
    }
}

#[derive(PartialEq,Clone,Debug)]
pub enum QueryParam {
    Bbox,
    Zoom,
    PixelWidth,
    ScaleDenominator,
}

#[derive(Clone,Debug)]
pub struct SqlQuery {
    pub sql: String,
    pub params: Vec<QueryParam>,
}

pub struct PostgisInput {
    pub connection_url: String,
    conn_pool: Option<r2d2::Pool<PostgresConnectionManager>>,
    // Queries for all layers and zoom levels
    queries: BTreeMap<String, BTreeMap<u8, SqlQuery>>,
}

impl SqlQuery {
    /// Replace variables (!bbox!, !zoom!, etc.) in query
    // https://github.com/mapnik/mapnik/wiki/PostGIS
    fn replace_params(&mut self, bbox_expr: String) {
        let mut numvars = 0;
        if self.sql.contains("!bbox!") {
            self.params.push(QueryParam::Bbox);
            numvars += 4;
            self.sql = self.sql.replace("!bbox!", &bbox_expr);
        }
        // replace e.g. !zoom! with $5
        for (var, par, cast) in vec![("!zoom!", QueryParam::Zoom, ""),
                                     ("!pixel_width!", QueryParam::PixelWidth, "FLOAT8"),
                                     ("!scale_denominator!",
                                      QueryParam::ScaleDenominator,
                                      "FLOAT8")] {
            if self.sql.contains(var) {
                self.params.push(par);
                numvars += 1;
                if cast != "" {
                    self.sql = self.sql.replace(var, &format!("${}::{}", numvars, cast));
                } else {
                    self.sql = self.sql.replace(var, &format!("${}", numvars));
                }
            }
        }
    }
    fn valid_sql_for_params(sql: &String) -> String {
        let mut query: String;
        query = sql.replace("!bbox!", "ST_MakeEnvelope(0,0,0,0,3857)");
        query = query.replace("!zoom!", "0");
        query = query.replace("!pixel_width!", "0");
        query = query.replace("!scale_denominator!", "0");
        query
    }
}

impl PostgisInput {
    pub fn new(connection_url: &str) -> PostgisInput {
        PostgisInput {
            connection_url: connection_url.to_string(),
            conn_pool: None,
            queries: BTreeMap::new(),
        }
    }
    /// New instance with connected pool
    pub fn connected(&self) -> PostgisInput {
        let manager = PostgresConnectionManager::new(self.connection_url.as_ref(), TlsMode::None)
            .unwrap();
        let config = r2d2::Config::builder().pool_size(10).build();
        let pool = r2d2::Pool::new(config, manager).unwrap();
        PostgisInput {
            connection_url: self.connection_url.clone(),
            conn_pool: Some(pool),
            queries: BTreeMap::new(),
        }
    }
    pub fn conn(&self) -> r2d2::PooledConnection<PostgresConnectionManager> {
        let pool = self.conn_pool.as_ref().unwrap();
        //debug!("{:?}", pool);
        pool.get().unwrap()
    }
    pub fn detect_layers(&self, detect_geometry_types: bool) -> Vec<Layer> {
        info!("Detecting layers from geometry_columns");
        let mut layers: Vec<Layer> = Vec::new();
        let conn = self.conn();
        let sql = "SELECT * FROM geometry_columns ORDER BY f_table_schema,f_table_name DESC";
        for row in &conn.query(sql, &[]).unwrap() {
            let schema: String = row.get("f_table_schema");
            let table_name: String = row.get("f_table_name");
            let geometry_column: String = row.get("f_geometry_column");
            let srid: i32 = row.get("srid");
            let geomtype: String = row.get("type");
            let mut layer = Layer::new(&table_name);
            layer.table_name = if schema != "public" {
                Some(format!("{}.{}", schema, table_name))
            } else {
                Some(table_name.clone())
            };
            layer.geometry_field = Some(geometry_column.clone());
            layer.geometry_type = match &geomtype as &str {
                "GEOMETRY" => {
                    if detect_geometry_types {
                        let field = layer.geometry_field.as_ref().unwrap();
                        let table = layer.table_name.as_ref().unwrap();
                        let types = self.detect_geometry_types(&layer);
                        if types.len() == 1 {
                            debug!("Detected unique geometry type in '{}.{}': {}",
                                   table,
                                   field,
                                   &types[0]);
                            Some(types[0].clone())
                        } else {
                            let type_list = types.join(", ");
                            warn!("Multiple geometry types in '{}.{}': {}",
                                  table,
                                  field,
                                  type_list);
                            Some("GEOMETRY".to_string())
                        }
                    } else {
                        warn!("Unknwon geometry type of '{}.{}'",
                              table_name,
                              geometry_column);
                        Some("GEOMETRY".to_string())
                    }
                }
                _ => Some(geomtype.clone()),
            };
            layer.srid = Some(srid);
            layers.push(layer);
        }
        layers
    }
    pub fn detect_geometry_types(&self, layer: &Layer) -> Vec<String> {
        let field = layer.geometry_field.as_ref().unwrap();
        let table = layer.table_name.as_ref().unwrap();
        debug!("Detecting geometry types for field '{}' in table '{}'",
               field,
               table);

        let conn = self.conn();
        let sql = format!("SELECT DISTINCT GeometryType({}) AS geomtype FROM {}",
                          field,
                          table);

        let mut types: Vec<String> = Vec::new();
        for row in &conn.query(&sql, &[]).unwrap() {
            types.push(row.get("geomtype"));
        }
        types
    }
    // Return column field names and Rust compatible type conversion
    pub fn detect_columns(&self, layer: &Layer, sql: Option<&String>) -> Vec<(String, String)> {
        let mut query = match sql {
            Some(&ref userquery) => userquery.clone(),
            None => {
                format!("SELECT * FROM {}",
                        layer.table_name.as_ref().unwrap_or(&layer.name))
            }
        };
        query = SqlQuery::valid_sql_for_params(&query);
        let conn = self.conn();
        let stmt = conn.prepare(&query);
        match stmt {
            Err(e) => {
                error!("Layer '{}': {}", layer.name, e);
                vec![]
            }
            Ok(stmt) => {
                let cols: Vec<(String, String)> = stmt.columns()
                    .iter()
                    .map(|col| {
                        let name = col.name().to_string();
                        let cast = match col.type_() {
                            &Type::Varchar | &Type::Text | &Type::CharArray | &Type::Float4 |
                            &Type::Float8 | &Type::Int2 | &Type::Int4 | &Type::Int8 |
                            &Type::Bool => String::new(),
                            &Type::Numeric => "FLOAT8".to_string(),
                            &Type::Other(ref other) => {
                                match other.name() {
                                    "geometry" => String::new(),
                                    _ => "TEXT".to_string(),
                                }
                            }
                            _ => "TEXT".to_string(),
                        };
                        if !cast.is_empty() {
                            warn!("Layer '{}': Converting field '{}' of type {} to {}",
                                  layer.name,
                                  name,
                                  col.type_().name(),
                                  cast);
                        }
                        (name, cast)
                    })
                    .collect();
                let _ = stmt.finish();
                cols
            }
        }
    }
    // Return column field names and Rust compatible type conversion - without geometry column
    pub fn detect_data_columns(&self,
                               layer: &Layer,
                               sql: Option<&String>)
                               -> Vec<(String, String)> {
        debug!("detect_data_columns for layer {} with sql {:?}",
               layer.name,
               sql);
        let cols = self.detect_columns(layer, sql);
        let filter_cols = vec![layer.geometry_field.as_ref().unwrap()];
        cols.into_iter()
            .filter(|&(ref col, _)| !filter_cols.contains(&&col))
            .collect()
    }
    /// Build geometry selection expression for feature query.
    fn build_geom_expr(&self, layer: &Layer, grid_srid: i32, raw_geom: bool) -> String {
        let layer_srid = layer.srid.unwrap_or(0);
        let ref geom_name = layer.geometry_field.as_ref().unwrap();
        let mut geom_expr = String::from(geom_name as &str);

        if !raw_geom {
            // Clipping
            if layer.buffer_size.is_some() {
                match layer
                          .geometry_type
                          .as_ref()
                          .unwrap_or(&"GEOMETRY".to_string()) as &str {
                    "POLYGON" | "MULTIPOLYGON" => {
                        geom_expr = format!("ST_Buffer(ST_Intersection(ST_MakeValid({}),!bbox!), 0.0)",
                                            geom_expr);
                    }
                    _ => {
                        geom_expr = format!("ST_Intersection(ST_MakeValid({}),!bbox!)", geom_expr);
                    }
                    //Buffer is added to !bbox! when replaced
                };
            }

            // convert LINESTRING and POLYGON to multi geometries (and fix potential (empty) single types)
            match layer
                      .geometry_type
                      .as_ref()
                      .unwrap_or(&"GEOMETRY".to_string()) as &str {
                "LINESTRING" |
                "MULTILINESTRING" |
                "POLYGON" |
                "MULTIPOLYGON" => {
                    geom_expr = format!("ST_Multi({})", geom_expr);
                }
                _ => {}
            }

            // Simplify
            if layer.simplify.unwrap_or(false) {
                geom_expr = match layer
                          .geometry_type
                          .as_ref()
                          .unwrap_or(&"GEOMETRY".to_string()) as
                                  &str {
                    "LINESTRING" |
                    "MULTILINESTRING" => {
                        format!("ST_Multi(ST_SimplifyPreserveTopology({},!pixel_width!/2))",
                                geom_expr)
                    }
                    "POLYGON" | "MULTIPOLYGON" => {
                        let empty_geom = format!("ST_GeomFromText('MULTIPOLYGON EMPTY',{})",
                                                 layer_srid);
                        format!("COALESCE(ST_SnapToGrid({}, !pixel_width!/2),{})::geometry(MULTIPOLYGON,{})",
                                geom_expr,
                                empty_geom,
                                layer_srid)
                    }
                    _ => geom_expr, // No simplification for points or unknown types
                };
            }

        }

        // Transform geometry to grid SRID
        if layer_srid <= 0 {
            // Unknown SRID
            warn!("Layer '{}' - Casting geometry '{}' to SRID {}",
                  layer.name,
                  geom_name,
                  grid_srid);
            geom_expr = format!("ST_SetSRID({},{})", geom_expr, grid_srid)
        } else if layer_srid != grid_srid {
            warn!("Layer '{}' - Reprojecting geometry '{}' to SRID {}",
                  layer.name,
                  geom_name,
                  grid_srid);
            geom_expr = format!("ST_Transform({},{})", geom_expr, grid_srid);
        }

        if geom_expr.starts_with("ST_") || geom_expr.starts_with("COALESCE") {
            geom_expr = format!("{} AS {}", geom_expr, geom_name);
        }

        geom_expr
    }
    /// Build select list expressions for feature query.
    fn build_select_list(&self, layer: &Layer, geom_expr: String, sql: Option<&String>) -> String {
        let offline = self.conn_pool.is_none();
        if offline {
            geom_expr
        } else {
            let mut cols: Vec<String> = self.detect_data_columns(layer, sql)
                .iter()
                .map(|&(ref name, ref casttype)| {
                         // Wrap column names in double quotes to guarantee validity. Columns might have colons
                         if casttype.is_empty() {
                             format!("\"{}\"", name)
                         } else {
                             format!("\"{}\"::{}", name, casttype)
                         }
                     })
                .collect();
            cols.insert(0, geom_expr);
            cols.join(",")
        }
    }
    /// Build !bbox! replacement expression for feature query.
    fn build_bbox_expr(&self, layer: &Layer, grid_srid: i32) -> String {
        let layer_srid = layer.srid.unwrap_or(grid_srid); // we assume grid srid as default
        let env_srid = if layer_srid <= 0 {
            layer_srid
        } else {
            grid_srid
        };
        let mut expr;
        expr = format!("ST_MakeEnvelope($1,$2,$3,$4,{})", env_srid);
        if let Some(pixels) = layer.buffer_size {
            expr = format!("ST_Buffer({},{}*!pixel_width!)", expr, pixels);
        }
        if layer_srid > 0 && layer_srid != grid_srid {
            expr = format!("ST_Transform({},{})", expr, layer_srid);
        };
        expr
    }
    /// Build feature query SQL (also used for generated config).
    pub fn build_query_sql(&self,
                           layer: &Layer,
                           grid_srid: i32,
                           sql: Option<&String>,
                           raw_geom: bool)
                           -> Option<String> {
        let mut query;
        let offline = self.conn_pool.is_none();
        let geom_expr = self.build_geom_expr(layer, grid_srid, raw_geom);
        let select_list = self.build_select_list(layer, geom_expr, sql);
        let intersect_clause = format!(" WHERE {} && !bbox!",
                                       layer.geometry_field.as_ref().unwrap());

        if let Some(&ref userquery) = sql {
            // user query
            let ref select = if offline {
                "*".to_string()
            } else {
                select_list
            };
            query = format!("SELECT {} FROM ({}) AS _q", select, userquery);
            if !userquery.contains("!bbox!") {
                query.push_str(&intersect_clause);
            }
        } else {
            // automatic query
            //TODO: check min-/maxzoom + handle overzoom
            if layer.table_name.is_none() {
                return None;
            }
            query = format!("SELECT {} FROM {}",
                            select_list,
                            layer.table_name.as_ref().unwrap());
            query.push_str(&intersect_clause);
        };

        Some(query)
    }
    pub fn build_query(&self,
                       layer: &Layer,
                       grid_srid: i32,
                       sql: Option<&String>)
                       -> Option<SqlQuery> {
        let sqlquery = self.build_query_sql(layer, grid_srid, sql, false);
        if sqlquery.is_none() {
            return None;
        }
        let mut sqlquery = sqlquery.unwrap();
        if let Some(n) = layer.query_limit {
            sqlquery.push_str(&format!(" LIMIT {}", n));
        }
        let bbox_expr = self.build_bbox_expr(layer, grid_srid);
        let mut query = SqlQuery {
            sql: sqlquery,
            params: Vec::new(),
        };
        query.replace_params(bbox_expr);
        Some(query)
    }
    pub fn prepare_queries(&mut self, layer: &Layer, grid_srid: i32) {
        let mut queries = BTreeMap::new();

        for layer_query in &layer.query {
            if let Some(query) = self.build_query(layer, grid_srid, layer_query.sql.as_ref()) {
                debug!("Query for layer '{}': {}", layer.name, query.sql);
                for zoom in layer_query.minzoom()..layer_query.maxzoom() {
                    if &layer.query(zoom).unwrap_or(&"".to_string()) ==
                       &layer_query.sql.as_ref().unwrap_or(&"".to_string()) {
                        queries.insert(zoom, query.clone());
                    }
                }
            }
        }

        let has_gaps = (layer.minzoom()..layer.maxzoom()).any(|zoom| !queries.contains_key(&zoom));

        // Genereate queries for zoom levels without user sql
        if has_gaps {
            if let Some(query) = self.build_query(layer, grid_srid, None) {
                debug!("Query for layer '{}': {}", layer.name, query.sql);
                for zoom in layer.minzoom()..layer.maxzoom() {
                    if !queries.contains_key(&zoom) {
                        queries.insert(zoom, query.clone());
                    }
                }
            }
        }

        self.queries.insert(layer.name.clone(), queries);
    }
    fn query(&self, layer: &Layer, zoom: u8) -> Option<&SqlQuery> {
        let ref queries = self.queries[&layer.name];
        Some(&queries[&zoom])
    }
}

impl DatasourceInput for PostgisInput {
    fn retrieve_features<F>(&self,
                            layer: &Layer,
                            extent: &Extent,
                            zoom: u8,
                            grid: &Grid,
                            mut read: F)
        where F: FnMut(&Feature)
    {
        let conn = self.conn();
        let query = self.query(&layer, zoom);
        if query.is_none() {
            return;
        }
        let query = query.unwrap();
        let stmt = conn.prepare_cached(&query.sql);
        if let Err(err) = stmt {
            error!("Layer '{}': {}", layer.name, err);
            error!("Query: {}", query.sql);
            return;
        };

        // Add query params
        let zoom_param = zoom as i16;
        let pixel_width = grid.pixel_width(zoom); //TODO: calculate only if needed
        let scale_denominator = grid.scale_denominator(zoom);
        let mut params = Vec::new();
        for param in &query.params {
            match param {
                &QueryParam::Bbox => {
                    let mut bbox: Vec<&ToSql> =
                        vec![&extent.minx, &extent.miny, &extent.maxx, &extent.maxy];
                    params.append(&mut bbox);
                }
                &QueryParam::Zoom => params.push(&zoom_param),
                &QueryParam::PixelWidth => params.push(&pixel_width),
                &QueryParam::ScaleDenominator => {
                    //NOTE: function z() in osm2vectortiles takes numeric argument, which is not
                    // supported by rust postgresql
                    params.push(&scale_denominator);
                }
            }
        }

        let stmt = stmt.unwrap();
        let rows = stmt.query(&params.as_slice());
        if let Err(err) = rows {
            error!("Layer '{}': {}", layer.name, err);
            error!("Query: {}", query.sql);
            error!("Param types: {:?}", query.params);
            error!("Param values: {:?}", params);
            return;
        };
        debug!("Reading features in layer {}", layer.name); // rust_postgis may panic with unexpected geometry data
        for row in &rows.unwrap() {
            let feature = FeatureRow {
                layer: layer,
                row: &row,
            };
            read(&feature);
        }
    }
}

impl Config<PostgisInput> for PostgisInput {
    fn from_config(config: &toml::Value) -> Result<Self, String> {
      match env::var("TREX_DATASOURCE_URL") {
        Ok(url) => Ok(PostgisInput::new(url.as_str())),
        Err(_) => config
          .get("datasource")
          .and_then(|d| d.get("url"))
          .ok_or("Missing configuration entry 'datasource.url'".to_string())
          .and_then(|val| {
              val.as_str()
              .ok_or("url entry is not a string".to_string())
              })
          .and_then(|url| Ok(PostgisInput::new(url)))
      }
    }

    fn gen_config() -> String {
        let toml = r#"
[datasource]
type = "postgis"
# Connection specification (https://github.com/sfackler/rust-postgres#connecting)
url = "postgresql://user:pass@host/database"
"#;
        toml.to_string()
    }
    fn gen_runtime_config(&self) -> String {
        format!(r#"
[datasource]
type = "postgis"
url = "{}"
"#,
                self.connection_url)
    }
}
