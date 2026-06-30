use crate::error::AirpError;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// M5.6：单个 PNG chunk 的最大允许字节数（16 MiB）。
/// 防御恶意 PNG 声明巨型 chunk 长度造成 OOM；正常角色卡 chara 块远低于此。
const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// zTXt / 压缩 iTXt 解压后的字节上限（64 MiB）。
/// 压缩流可放大解压体积（zip bomb），用 `take` 截断防 OOM。
const MAX_INFLATED_SIZE: u64 = 64 * 1024 * 1024;

/// zlib 解压（zTXt / 压缩 iTXt 用 deflate，PNG 规范唯一压缩方法 0）。
/// 超过 [`MAX_INFLATED_SIZE`] 即视为失败，返回 None。
fn inflate_zlib(data: &[u8]) -> Option<String> {
    use flate2::read::ZlibDecoder;
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .take(MAX_INFLATED_SIZE)
        .read_to_end(&mut out)
        .ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// 从 PNG 文件中提取角色卡 JSON 数据。
/// 支持 `tEXt` 和未压缩 `iTXt` 文本块；键名 `ccv3`(V3) 优先，回退 `chara`(V2)。
pub fn parse_png_character_card<P: AsRef<Path>>(path: P) -> Result<String, AirpError> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    parse_png_character_card_bytes(&bytes)
}

/// 同 [`parse_png_character_card`]，但直接从内存字节解析（不触磁盘）。
/// 用于 import 边界：先 decode + 校验形状再决定是否落盘，避免脏文件残留。
pub fn parse_png_character_card_bytes(bytes: &[u8]) -> Result<String, AirpError> {
    use std::io::Cursor;
    let mut file = Cursor::new(bytes);

    // 1. 验证 PNG 头部签名
    let mut signature = [0u8; 8];
    file.read_exact(&mut signature)?;
    if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(AirpError::BadRequest("非法 PNG 文件签名".to_string()));
    }

    // 2. 收集候选文本块。V3 卡把完整数据存在 `ccv3` 关键字，并保留 `chara`
    //    (V2 降级视图) 作兼容；故扫描全部文本块，ccv3 优先于 chara。
    let mut ccv3: Option<String> = None;
    let mut chara: Option<String> = None;

    loop {
        let mut length_buf = [0u8; 4];
        if file.read_exact(&mut length_buf).is_err() {
            // 到达文件末尾
            break;
        }
        let length = u32::from_be_bytes(length_buf) as usize;
        if length > MAX_CHUNK_SIZE {
            return Err(AirpError::BadRequest(format!(
                "PNG chunk 过大：{} 字节（上限 {} 字节）",
                length, MAX_CHUNK_SIZE
            )));
        }

        let mut type_buf = [0u8; 4];
        file.read_exact(&mut type_buf)?;
        let chunk_type = String::from_utf8_lossy(&type_buf).into_owned();

        // 读取 Chunk Data
        let mut data = vec![0u8; length];
        file.read_exact(&mut data)?;

        // 读取 CRC (跳过)
        let mut crc_buf = [0u8; 4];
        file.read_exact(&mut crc_buf)?;

        // 判断类型：tEXt（明文）/ zTXt（zlib 压缩）/ iTXt（明文或压缩）。
        let kv = if chunk_type == "tEXt" {
            parse_text_chunk(&data)
        } else if chunk_type == "zTXt" {
            parse_ztxt_chunk(&data)
        } else if chunk_type == "iTXt" {
            parse_itxt_chunk(&data)
        } else if chunk_type == "IEND" {
            break;
        } else {
            None
        };

        if let Some((keyword, text)) = kv {
            match keyword.as_str() {
                "ccv3" if ccv3.is_none() => ccv3 = Some(text),
                "chara" if chara.is_none() => chara = Some(text),
                _ => {}
            }
        }

        // ccv3 是最高优先级，拿到即可停止扫描。
        if ccv3.is_some() {
            break;
        }
    }

    if let Some(text) = ccv3.or(chara) {
        return Ok(decode_chara_data(&text));
    }

    Err(AirpError::BadRequest(
        "未在 PNG 文件中找到 chara/ccv3 角色卡数据".to_string(),
    ))
}

fn parse_text_chunk(data: &[u8]) -> Option<(String, String)> {
    let null_pos = data.iter().position(|&b| b == 0)?;
    let keyword = String::from_utf8_lossy(&data[..null_pos]).into_owned();
    let text = String::from_utf8_lossy(&data[null_pos + 1..]).into_owned();
    Some((keyword, text))
}

/// zTXt 布局：keyword \0 compression_method(1B) compressed_text。
/// PNG 规范仅定义压缩方法 0（zlib/deflate）。
fn parse_ztxt_chunk(data: &[u8]) -> Option<(String, String)> {
    let null_pos = data.iter().position(|&b| b == 0)?;
    let keyword = String::from_utf8_lossy(&data[..null_pos]).into_owned();
    let rest = data.get(null_pos + 1..)?;
    let (&method, compressed) = rest.split_first()?;
    if method != 0 {
        return None;
    }
    let text = inflate_zlib(compressed)?;
    Some((keyword, text))
}

fn parse_itxt_chunk(data: &[u8]) -> Option<(String, String)> {
    let k_null = data.iter().position(|&b| b == 0)?;
    let keyword = String::from_utf8_lossy(&data[..k_null]).into_owned();

    let idx = k_null + 1;
    if idx + 2 > data.len() {
        return None;
    }
    let compression_flag = data[idx];

    let lang_start = idx + 2;
    let lang_null = data[lang_start..].iter().position(|&b| b == 0)? + lang_start;

    let trans_start = lang_null + 1;
    let trans_null = data[trans_start..].iter().position(|&b| b == 0)? + trans_start;

    let text_bytes = &data[trans_null + 1..];

    // compression_flag==1：text 经压缩，方法位（idx+1）为 0 时是 zlib/deflate。
    let text = if compression_flag == 1 {
        if data[idx + 1] != 0 {
            return None;
        }
        inflate_zlib(text_bytes)?
    } else {
        String::from_utf8_lossy(text_bytes).into_owned()
    };
    Some((keyword, text))
}

fn decode_chara_data(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(decoded_bytes) = STANDARD.decode(trimmed) {
        if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
            return decoded_str;
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// 构造一个 PNG 头 + 单个 chunk 头（长度 + 类型）的字节流。
    /// `length` 写入 chunk 长度字段；不会真的填这么多字节（用于触发上限检查）。
    fn write_png_with_chunk_length(length: u32, chunk_type: &[u8; 4]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        // PNG 签名
        f.write_all(&[137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
        // chunk length
        f.write_all(&length.to_be_bytes()).unwrap();
        // chunk type
        f.write_all(chunk_type).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_rejects_oversized_chunk() {
        // 17 MiB chunk → 应被上限拒绝，不分配 vec
        let f = write_png_with_chunk_length(17 * 1024 * 1024, b"tEXt");
        let res = parse_png_character_card(f.path());
        assert!(res.is_err(), "expected size-limit rejection, got {:?}", res);
        let msg = res.unwrap_err().to_string();
        assert!(
            msg.contains("过大") || msg.contains("上限"),
            "unexpected error msg: {}",
            msg
        );
    }

    #[test]
    fn test_rejects_bad_signature() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"NOTAPNGFILE").unwrap();
        f.flush().unwrap();
        assert!(parse_png_character_card(f.path()).is_err());
    }

    /// 构造 PNG：签名 + 若干 tEXt 块 + IEND。CRC 填 0（解析器跳过 CRC）。
    fn write_png_with_text_chunks(chunks: &[(&str, &str)]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
        for (keyword, text) in chunks {
            let mut data = Vec::new();
            data.extend_from_slice(keyword.as_bytes());
            data.push(0);
            data.extend_from_slice(text.as_bytes());
            f.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
            f.write_all(b"tEXt").unwrap();
            f.write_all(&data).unwrap();
            f.write_all(&[0u8; 4]).unwrap(); // CRC（解析器跳过）
        }
        f.write_all(&0u32.to_be_bytes()).unwrap();
        f.write_all(b"IEND").unwrap();
        f.write_all(&[0u8; 4]).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_ccv3_preferred_over_chara() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let v2 = STANDARD.encode(r#"{"spec":"chara_card_v2","data":{"name":"v2"}}"#);
        let v3 = STANDARD.encode(r#"{"spec":"chara_card_v3","data":{"name":"v3"}}"#);
        // chara 在前、ccv3 在后，验证仍取 ccv3（最高优先级）。
        let f = write_png_with_text_chunks(&[("chara", &v2), ("ccv3", &v3)]);
        let out = parse_png_character_card(f.path()).unwrap();
        assert!(out.contains("chara_card_v3"), "应优先 ccv3，实际: {out}");
        assert!(!out.contains("chara_card_v2"));
    }

    #[test]
    fn test_chara_fallback_when_no_ccv3() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let v2 = STANDARD.encode(r#"{"spec":"chara_card_v2","data":{"name":"v2"}}"#);
        let f = write_png_with_text_chunks(&[("chara", &v2)]);
        let out = parse_png_character_card(f.path()).unwrap();
        assert!(out.contains("chara_card_v2"));
    }

    fn zlib_compress(s: &str) -> Vec<u8> {
        use flate2::{write::ZlibEncoder, Compression};
        use std::io::Write;
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(s.as_bytes()).unwrap();
        e.finish().unwrap()
    }

    /// 写一个 zTXt 块（keyword \0 method(0) zlib-data）+ IEND。
    fn write_png_with_ztxt(keyword: &str, text: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(keyword.as_bytes());
        data.push(0); // keyword 终止符
        data.push(0); // compression method = 0 (zlib)
        data.extend_from_slice(&zlib_compress(text));
        f.write_all(&(data.len() as u32).to_be_bytes()).unwrap();
        f.write_all(b"zTXt").unwrap();
        f.write_all(&data).unwrap();
        f.write_all(&[0u8; 4]).unwrap();
        f.write_all(&0u32.to_be_bytes()).unwrap();
        f.write_all(b"IEND").unwrap();
        f.write_all(&[0u8; 4]).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_ztxt_compressed_chara() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let v3 = STANDARD.encode(r#"{"spec":"chara_card_v3","data":{"name":"zt"}}"#);
        let f = write_png_with_ztxt("ccv3", &v3);
        let out = parse_png_character_card(f.path()).unwrap();
        assert!(out.contains("chara_card_v3"), "zTXt 解压失败: {out}");
    }
}
