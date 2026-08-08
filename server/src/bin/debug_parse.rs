// 临时调试: 解析单个文件, reply 写入 out.txt (UTF-8)
use super_instruct_server::extensions::sse_parser::UniversalSseParser;
use super_instruct_server::core::ResponseParser;
use bytes::Bytes;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let raw = std::fs::read(path).expect("read");
    let parsed = UniversalSseParser.parse(&Bytes::from(raw));
    std::fs::write("out.txt", format!("THINKING:\n{}\n\nREPLY:\n{}", parsed.thinking, parsed.reply)).expect("write");
    println!("done, reply_len={}", parsed.reply.len());
}
