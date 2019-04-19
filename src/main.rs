extern crate futures;
extern crate kuchiki;
extern crate md5;
extern crate regex;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate tendril;
extern crate tokio;

use std::env;
use std::io::BufReader;
use std::ops::Add;

use reqwest::r#async::*;
use tendril::fmt::Slice;
use tendril::stream::TendrilSink;
use tokio::prelude::future::{result, Either};
use tokio::prelude::*;

use crate::tendril::SliceExt;

#[derive(Deserialize)]
struct Modpack {
    files: Vec<Modfile>,
}

#[derive(Deserialize)]
struct Modfile {
    projectID: serde_json::Number,
    fileID: serde_json::Number,
}

struct MetaData {
    filename: String,
    md5: String,
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let path: &str = &args[1];
    let file = std::fs::File::open(path).expect("opening manifest.json failed!");
    println!("Manifest opened!");
    let buf_reader = BufReader::new(file);
    let client = Client::new();
    let mod_pack: Modpack = serde_json::from_reader(buf_reader).unwrap();
    println!("Gathered data. Downloading modfiles...");
    tokio::run(futures::lazy(move || {
        for mod_file in mod_pack.files {
            let url = format!(
                "https://minecraft.curseforge.com/projects/{0}/files/{1}",
                mod_file.projectID, mod_file.fileID
            );
            tokio::spawn(
                gather_metadata(mod_file, &client, url)
                    .and_then(|(name_dom, md5, url)| {
                        check_integrity(name_dom, md5).and_then(
                            |(file_option, file_path_option): (
                                Option<tokio::fs::File>,
                                Option<String>,
                            )| {
                                download_mod_file(url, &client).and_then(|chunk| {
                                    //save_file(chunk, file_option, file_path_option).and_then(|_| Ok(()))
                                });
                            },
                        )
                    })
                    .map_err(|e| eprintln!("{:?}", e)),
            )
        }
    }));
}

fn gather_metadata(
    mod_file: Modfile,
    client: &Client,
    url: String,
) -> impl Future<Item = (String, String, String), Error = ()> {
    client
        .get(&url)
        .send()
        .and_then(|response| {
            response.into_body().from_err().concat2().and_then(|body| {
                let bytes = body.to_tendril();
                let html = kuchiki::parse_html().from_utf8().one(bytes);
                let name_dom = html
                    .select_first(".info-data .overflow-tip")
                    .unwrap()
                    .text_contents();
                let md5 = html.select_first(".md5").unwrap().text_contents();
                Ok((name_dom, md5, url))
            })
        })
        .map_err(|e| eprintln!("{:?}", e))
}

fn check_integrity(
    name_dom: String,
    md5: String,
) -> impl Future<Item = (Option<tokio::fs::File>, Option<String>), Error = ()> {
    tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .append(true)
        .open(format!("./mods/{}", &name_dom))
        .and_then(|mut file| {
            let mut file_bytes: Vec<u8> = vec![];
            file.read_buf(&mut file_bytes).and_then(|_| {
                if md5.as_bytes() == file_bytes.as_slice() {
                    Ok((None, None))
                } else {
                    Ok((Some(file), Some(name_dom)))
                }
            })
        })
        .map_err(|e| eprintln!("{:?}", e))
}

fn download_mod_file(url: String, client: &Client) -> impl Future<Item = Chunk, Error = ()> {
    let url_to_download = url.add("/download");
    client
        .get(&url_to_download)
        .send()
        .and_then(|response| {
            response
                .into_body()
                .from_err()
                .concat2()
                .and_then(|body| Ok(body))
        })
        .map_err(|e| eprintln!("{:?}", e))
}

/* fn save_file(
    file_body: Chunk,
    file_option: Option<tokio::fs::File>,
    file_path_option: Option<String>,
) -> impl Future<Item = (), Error = std::io::Error> {
    if file_option.is_some() {
        let mut file = file_option.unwrap();
        let file_path = file_path_option.unwrap();
        file.poll_write(&*file_body).and_then(|_| {
            println!("{} <- Done!", file_path);
            Ok(())
        })
    } else {
        Ok(())
    }
} */
