//! 子プロセスの出力に混じった秘密を伏せ字にする（1Password の `op run` と同じ考え方）。
//!
//! 出力は任意の位置で分割されて届くので、秘密の途中で切れたときに素通りさせないよう、
//! 「秘密の先頭部分かもしれない末尾」だけを次の塊まで持ち越す。

pub const MASK: &[u8] = b"<concealed by vlt>";
/// これより短い値は伏せない（1〜3 文字を伏せると出力のあちこちが潰れて読めなくなる）。
pub const MIN_MASK_LEN: usize = 4;

pub struct Masker {
    /// 長い順（長い秘密が短い秘密を含む場合に長い方を優先する）。
    secrets: Vec<Vec<u8>>,
    pending: Vec<u8>,
}

impl Masker {
    pub fn new<I, S>(secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u8]>,
    {
        let mut secrets: Vec<Vec<u8>> = secrets
            .into_iter()
            .map(|s| s.as_ref().to_vec())
            .filter(|s| s.len() >= MIN_MASK_LEN)
            .collect();
        secrets.sort_by(|a, b| b.len().cmp(&a.len()));
        secrets.dedup();
        Self { secrets, pending: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// 届いた塊を受け取り、今のうちに出してよいバイト列を返す。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(chunk);
        let (out, keep_from) = self.scan(false);
        self.pending.drain(..keep_from);
        out
    }

    /// 出力の終わり。持ち越していた分も伏せ字処理して出す。
    pub fn finish(&mut self) -> Vec<u8> {
        let (out, _) = self.scan(true);
        self.pending.clear();
        out
    }

    fn scan(&self, at_end: bool) -> (Vec<u8>, usize) {
        let buf = &self.pending;
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        'outer: while i < buf.len() {
            let rest = &buf[i..];
            for secret in &self.secrets {
                if rest.starts_with(secret) {
                    out.extend_from_slice(MASK);
                    i += secret.len();
                    continue 'outer;
                }
            }
            if !at_end && self.secrets.iter().any(|s| s.len() > rest.len() && s.starts_with(rest)) {
                // 秘密の書き出しかもしれないので次の塊を待つ
                return (out, i);
            }
            out.push(buf[i]);
            i += 1;
        }
        (out, i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(secrets: &[&str], chunks: &[&str]) -> String {
        let mut m = Masker::new(secrets.iter().map(|s| s.as_bytes()));
        let mut out = Vec::new();
        for c in chunks {
            out.extend(m.push(c.as_bytes()));
        }
        out.extend(m.finish());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn masks_every_occurrence() {
        assert_eq!(run(&["s3cret"], &["a s3cret b s3cret\n"]), "a <concealed by vlt> b <concealed by vlt>\n");
    }

    #[test]
    fn masks_secrets_split_across_chunks() {
        assert_eq!(run(&["s3cret"], &["token=s3", "cr", "et;"]), "token=<concealed by vlt>;");
        // 1 バイトずつ届いても伏せる
        let chunks: Vec<String> = "x=s3cret!".chars().map(String::from).collect();
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        assert_eq!(run(&["s3cret"], &refs), "x=<concealed by vlt>!");
    }

    #[test]
    fn false_prefix_is_released() {
        assert_eq!(run(&["s3cret"], &["s3c", "ond\n"]), "s3cond\n");
        assert_eq!(run(&["s3cret"], &["ends with s3c"]), "ends with s3c");
    }

    #[test]
    fn longer_secret_wins_over_contained_shorter_one() {
        assert_eq!(run(&["abcd", "abcdefgh"], &["[abcdefgh][abcd]"]), "[<concealed by vlt>][<concealed by vlt>]");
    }

    #[test]
    fn short_and_empty_values_are_not_masked() {
        let m = Masker::new(["", "abc"]);
        assert!(m.is_empty());
        assert_eq!(run(&["abc"], &["abc"]), "abc");
    }

    #[test]
    fn multiline_and_utf8_secrets() {
        assert_eq!(run(&["line1\nline2"], &["<line1\nli", "ne2>"]), "<<concealed by vlt>>");
        assert_eq!(run(&["秘密の値"], &["値=秘密", "の値"]), "値=<concealed by vlt>");
    }

    #[test]
    fn output_without_secrets_streams_immediately() {
        let mut m = Masker::new(["zzzz"]);
        assert_eq!(m.push(b"hello\n"), b"hello\n");
    }
}
