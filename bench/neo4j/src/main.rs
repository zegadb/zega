// bench: the zega vs Neo4j benchmark client. See README.md.
//
//   bench gen <nodes> <dir>
//   bench load-zega --dataset <dir> --data <data-dir>
//   bench load-neo4j --dataset <dir> --url <host:port> --password <pw> [--clear]
//   bench parity --zega-url <url> --neo4j-url <host:port> --password <pw> --dataset <dir> [--samples N]
//   bench run --engine zega|neo4j|neo4j-http --url <url> --password <pw> --dataset <dir> \
//        --query lookup|onehop|twohop|filtered|path|write --conc <n> --seconds <n> \
//        [--warmup s] [--write-base n] [--tag s]

mod bench;
mod gen;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => gen::gen(&args[1..]),
        Some("load-zega") => bench::load_zega(&args[1..]),
        Some("load-neo4j") => bench::load_neo4j(&args[1..]).await,
        Some("parity") => bench::parity_command(&args[1..]).await,
        Some("run") => bench::run_command(&args[1..]).await,
        _ => gen::usage(),
    }
}
