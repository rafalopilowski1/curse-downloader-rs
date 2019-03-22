extern crate futures;
extern crate regex;
extern crate serde;
extern crate tokio;

#[macro_use]
extern crate serde_derive;
extern crate reqwest;
extern crate serde_json;

use reqwest::r#async::*;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio::prelude::*;

#[derive(Deserialize)]
struct Modpack {
    files: Vec<Modfile>,
}

#[derive(Deserialize)]
struct Modfile {
    projectID: serde_json::Number,
    fileID: serde_json::Number,
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let path: &str = &args[1];
    let file = File::open(path).expect("opening manifest.json failed!");
    println!("Manifest opened!");
    let buf_reader = BufReader::new(file);
    let mod_pack: Modpack = serde_json::from_reader(buf_reader).unwrap();
    println!("Gathered data. Downloading modfiles...");
    let client = Arc::new(Client::new());
    tokio::run(futures::lazy(move || {
        for mod_file in mod_pack.files {
            tokio::spawn(
                execute_download(mod_file, client.clone())
                    .and_then(|(chunk, path)| save_file(chunk, path))
                    .map_err(|e| {
                        println!("{:?}", e);
                    }),
            );
        }
        Ok(())
    }));
}

fn execute_download(
    mod_file: Modfile,
    client: Arc<Client>,
) -> impl Future<Item = (Chunk, String), Error = ()> {
    let url = format!(
        "https://minecraft.curseforge.com/projects/{0}/files/{1}/download",
        mod_file.projectID, mod_file.fileID
    );
    client
        .get(&url)
        .send()
        .and_then(|response| {
            let regex = regex::Regex::new("[^/]*$").unwrap();
            let mat = format!(
                "./{}",
                regex.find(response.url().as_str()).unwrap().as_str()
            );
            response
                .into_body()
                .concat2()
                .from_err()
                .and_then(|chunk| Ok((chunk, mat)))
        })
        .map_err(|e| eprintln!("{:?}", e))
}

fn save_file(chunk: Chunk, path: String) -> impl Future<Item = (), Error = ()> {
    tokio::fs::File::create(path.clone())
        .and_then(move |mut file| {
            file.write_all(&chunk).and_then(|_| {
                println!("{} <- Done!", path);
                Ok(())
            })
        })
        .map_err(|e| eprintln!("{:?}", e))
}
