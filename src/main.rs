use futures::{SinkExt, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::{
    borrow::Cow,
    env,
    io::{ErrorKind, Sink},
    pin::Pin,
    sync::Arc,
};
use tokio::io::*;
use tokio_util::*;

#[derive(Deserialize)]
struct Modpack {
    files: Vec<Modfile>,
}

#[derive(Deserialize, Clone)]
struct Modfile {
    projectID: serde_json::Number,
    fileID: serde_json::Number,
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = env::args().collect();
    let path: &str = &args[1];
    let file = std::fs::File::open(path).expect("File error");
    println!("Manifest opened!");
    if let Err(e) = std::fs::create_dir("mods") {
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => println!("'mods' folder already exists..."),
            _ => eprintln!("Error: {:?}", e.to_string()),
        }
    }
    let buf_reader = std::io::BufReader::new(file);
    let client = Client::new();
    let mod_pack: Modpack = serde_json::from_reader(buf_reader).expect("Parsing error!");
    println!("Gathered data. Downloading modfiles...");
    futures::stream::iter(mod_pack.files)
        .for_each_concurrent(None, |mod_file| {
            let client = client.clone();
            async move {
                let response = get_mod_file_response(&mod_file, &client).await;
                let mod_to_save = create_file("mod.jar").await.expect("creating file error");
                save_mod_to_file(response, mod_to_save).await
            }
        })
        .await;
}

async fn save_mod_to_file(response: reqwest::Response, mod_to_save: tokio::fs::File) {
    let mut file_stream = response
        .bytes_stream()
        .map_err(|_| std::io::Error::new(ErrorKind::Other, ""));
    let buf_writer = tokio::io::BufWriter::new(mod_to_save);
    let mut sink =
        tokio_util::codec::FramedWrite::new(buf_writer, tokio_util::codec::BytesCodec::new());
    sink.send_all(&mut file_stream).await;
}

async fn get_mod_file_response(mod_file: &Modfile, client: &Client) -> reqwest::Response {
    let url = format!(
        "https://minecraft.curseforge.com/projects/{0}/download/{1}/file",
        mod_file.projectID, mod_file.fileID
    );
    let request = client.clone().get(url).build().expect("request error");
    let response = client.execute(request).await.expect("response error");
    response
}
async fn create_file<S>(path: S) -> Result<tokio::fs::File>
where
    S: AsRef<str>,
{
    tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path.as_ref())
        .await
}
