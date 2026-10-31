use crate::{Mesh, Part, Transform, Unit, invalid};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek};
use std::path::Path;

#[derive(Default)]
struct Object {
    part: Part,
    components: Vec<(u32, Transform)>,
}

type Attributes = HashMap<String, String>;

fn attributes(event: &BytesStart<'_>) -> io::Result<Attributes> {
    event
        .attributes()
        .map(|a| {
            let a = a.map_err(|e| invalid(e.to_string()))?;
            let name = a.key.as_ref().to_owned();
            let value = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|e| invalid(e.to_string()))?
                .into_owned();
            Ok((name, value))
        })
        .collect()
}

fn required<T: std::str::FromStr>(attrs: &Attributes, name: &str) -> io::Result<T> {
    attrs
        .get(name)
        .ok_or_else(|| invalid(format!("3MF missing {name}")))?
        .parse()
        .map_err(|_| invalid(format!("3MF invalid {name}")))
}

fn transform(attrs: &Attributes) -> io::Result<Transform> {
    let Some(value) = attrs.get("transform") else {
        return Ok(Transform::IDENTITY);
    };
    let v = value
        .split_whitespace()
        .map(|s| {
            s.parse::<f64>()
                .map_err(|_| invalid("3MF invalid transform"))
        })
        .collect::<io::Result<Vec<_>>>()?;
    if v.len() != 12 {
        return Err(invalid("3MF transform requires 12 numbers"));
    }
    let result = Transform([
        [v[0], v[3], v[6], v[9]],
        [v[1], v[4], v[7], v[10]],
        [v[2], v[5], v[8], v[11]],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    result.validate()?;
    Ok(result)
}

pub(super) fn read(path: &Path) -> io::Result<(Mesh, Unit)> {
    read_from(File::open(path)?)
}

pub(super) fn read_from(reader: impl Read + Seek) -> io::Result<(Mesh, Unit)> {
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| invalid(e.to_string()))?;
    let mut relationships = String::new();
    archive
        .by_name("_rels/.rels")
        .map_err(|e| invalid(e.to_string()))?
        .read_to_string(&mut relationships)?;
    let mut reader = Reader::from_str(&relationships);
    let mut target = None;
    loop {
        match reader.read_event().map_err(|e| invalid(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "Relationship" => {
                let attrs = attributes(&e)?;
                if attrs.get("Type").is_some_and(|s| s.ends_with("/3dmodel")) {
                    target = attrs.get("Target").cloned();
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let target = target.ok_or_else(|| invalid("3MF package has no primary model relationship"))?;
    let mut xml = String::new();
    archive
        .by_name(target.trim_start_matches('/'))
        .map_err(|e| invalid(e.to_string()))?
        .read_to_string(&mut xml)?;
    let mut reader = Reader::from_str(&xml);
    let mut objects = HashMap::new();
    let mut current: Option<(u32, Object)> = None;
    let mut build = Vec::new();
    let mut unit = Unit::Millimeter;
    loop {
        match reader.read_event().map_err(|e| invalid(e.to_string()))? {
            Event::Start(e) | Event::Empty(e) => {
                let attrs = attributes(&e)?;
                match e.local_name().as_ref() {
                    "model" => {
                        if let Some(u) = attrs.get("unit") {
                            unit = u.parse()?;
                        }
                        if attrs
                            .get("requiredextensions")
                            .is_some_and(|s| !s.trim().is_empty())
                        {
                            return Err(invalid("3MF required extensions are not supported"));
                        }
                    }
                    "object" => {
                        let id = required(&attrs, "id")?;
                        let object = Object {
                            part: Part {
                                name: attrs
                                    .get("name")
                                    .cloned()
                                    .unwrap_or_else(|| format!("object-{id}")),
                                ..Part::default()
                            },
                            ..Object::default()
                        };
                        if current.is_some() {
                            return Err(invalid("nested 3MF objects"));
                        }
                        current = Some((id, object));
                    }
                    "vertex" => {
                        let object = &mut current
                            .as_mut()
                            .ok_or_else(|| invalid("3MF vertex outside object"))?
                            .1;
                        object.part.vertices.push([
                            required(&attrs, "x")?,
                            required(&attrs, "y")?,
                            required(&attrs, "z")?,
                        ]);
                    }
                    "triangle" => {
                        let object = &mut current
                            .as_mut()
                            .ok_or_else(|| invalid("3MF triangle outside object"))?
                            .1;
                        object.part.triangles.push([
                            required(&attrs, "v1")?,
                            required(&attrs, "v2")?,
                            required(&attrs, "v3")?,
                        ]);
                    }
                    "component" | "item" => {
                        if attrs.keys().any(|key| key.ends_with(":path")) {
                            return Err(invalid(
                                "3MF external component models require the production extension",
                            ));
                        }
                        let reference = (required(&attrs, "objectid")?, transform(&attrs)?);
                        if e.local_name().as_ref() == "component" {
                            current
                                .as_mut()
                                .ok_or_else(|| invalid("3MF component outside object"))?
                                .1
                                .components
                                .push(reference);
                        } else if attrs
                            .get("printable")
                            .is_none_or(|p| p != "0" && p != "false")
                        {
                            build.push(reference);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.local_name().as_ref() == "object" => {
                let (id, object) = current
                    .take()
                    .ok_or_else(|| invalid("3MF object end without start"))?;
                if objects.insert(id, object).is_some() {
                    return Err(invalid("duplicate 3MF object id"));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    fn instantiate(
        id: u32,
        world: Transform,
        prefix: &str,
        objects: &HashMap<u32, Object>,
        stack: &mut HashSet<u32>,
        result: &mut Mesh,
    ) -> io::Result<()> {
        if stack.len() > 256 || !stack.insert(id) {
            return Err(invalid("cyclic or excessive 3MF component nesting"));
        }
        let object = objects
            .get(&id)
            .ok_or_else(|| invalid("3MF references unknown object"))?;
        let name = format!("{prefix}/{}", object.part.name);
        if !object.part.triangles.is_empty() {
            let mut part = object.part.clone();
            part.name = name.clone();
            part.validate()?;
            part.transform(world)?;
            result.parts.push(part);
        }
        for &(child, transform) in &object.components {
            instantiate(
                child,
                world.compose(transform),
                &name,
                objects,
                stack,
                result,
            )?;
        }
        stack.remove(&id);
        Ok(())
    }
    let mut result = Mesh::default();
    for (i, (id, transform)) in build.into_iter().enumerate() {
        instantiate(
            id,
            transform,
            &format!("build-{i}"),
            &objects,
            &mut HashSet::new(),
            &mut result,
        )?;
    }
    Ok((result, unit))
}
