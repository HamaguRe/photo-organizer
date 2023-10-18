//! Exifデータの読み出しを行うためのモジュール
//! JPEGのみ

enum ByteOrder {
    BigEndian,
    LittleEndian,
}

/// APP1セグメント内におけるTIFFヘッダのオフセット
const OFFSET_TIFF_HEADER: usize = 10;

// タグ番号
const EXIF_IFD_POINTER: u16 = 0x8769;
const DATE_TIME_ORIGINAL: u16 = 0x9003;

/// APP0セグメントの次のセグメントの先頭のインデックスを返す。
pub fn next_app0_index(non_app1_binary: &[u8]) -> Result<usize, &'static str> {
    // JPEG画像先頭のSOIマーカを確認
    if non_app1_binary[..2] != [0xFF, 0xD8] {
        return Err("SOI marker does not exist.");
    }

    // APP0セグメントの終端を探す
    // APP0セグメントがない場合はSOIマーカの次のインデックスを返す。
    let mut next_app0 = 2;  // APP0の次のセグメント先頭を指すインデックス
    for i in 2..(non_app1_binary.len() - 1) {
        if non_app1_binary[i..=(i + 1)] == [0xFF, 0xE0] {  // APP0のマーカを探す
            // セグメント長は必ずビッグエンディアン
            let tmp_h = non_app1_binary[i + 2] as usize;
            let tmp_l = non_app1_binary[i + 3] as usize;
            let segment_len = tmp_h << 8 | tmp_l;  // セグメント長
            // ASCII文字も一応確認
            if &non_app1_binary[(i+4)..(i+9)] == b"JFIF\0" {
                next_app0 = i + segment_len + 2;
                break;
            }
        }
    }

    Ok(next_app0)
}

/// JPEG画像のバイナリデータのうちExifを格納した
/// APP1セグメント（マーカを含む）のスライスを返す
pub fn get_app1(binary: &[u8]) -> Option<&[u8]> {
    for i in 0..(binary.len() - 1) {
        if binary[i..=(i + 1)] == [0xFF, 0xE1] {  // APP1のマーカを探す
            // セグメント長は必ずビッグエンディアン
            let tmp_h = binary[i + 2] as usize;
            let tmp_l = binary[i + 3] as usize;
            let segment_len = tmp_h << 8 | tmp_l;  // セグメント長
            // Exif識別子を確認（XMPの可能性があるため）
            if &binary[(i+4)..(i+9)] == b"Exif\0" {
                return Some(&binary[i..(i + segment_len + 2)]);
            }
        }
    }
    None
}

/// DateTimeOriginalタグのvalueを返す（ASCII文字列で、終端のNULL文字は除く）。
/// 
/// Format: YYYY:MM:DD HH:MM:SS (Example: 2015:09:27 11:43:11)
pub fn get_date_time_original(app1: &[u8]) -> Option<[u8; 19]> {
    let byte_order = if app1[OFFSET_TIFF_HEADER..(OFFSET_TIFF_HEADER + 2)] == [0x4D, 0x4D] {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian
    };

    // 0th IFDのオフセットを読む。起点はExif識別子の直後（TIFFヘッダの先頭）
    let mut tmp = [0u8; 4];
    tmp.copy_from_slice(&app1[(OFFSET_TIFF_HEADER + 4)..(OFFSET_TIFF_HEADER + 8)]);
    let offset_0th_ifd = match byte_order {
        ByteOrder::BigEndian    => u32::from_be_bytes(tmp),
        ByteOrder::LittleEndian => u32::from_le_bytes(tmp),
    } as usize;

    // --- 0th IFDを読む ---
    let mut tmp = [0u8; 2];
    tmp.copy_from_slice(&app1[(OFFSET_TIFF_HEADER + offset_0th_ifd)..(OFFSET_TIFF_HEADER + offset_0th_ifd + 2)]);
    let tag_num = match byte_order {
        ByteOrder::BigEndian    => u16::from_be_bytes(tmp),
        ByteOrder::LittleEndian => u16::from_le_bytes(tmp),
    } as usize;

    // Exif IFD Pointerタグを探す
    let tag = match byte_order {
        ByteOrder::BigEndian    => EXIF_IFD_POINTER.to_be_bytes(),
        ByteOrder::LittleEndian => EXIF_IFD_POINTER.to_le_bytes(),
    };
    let tag_offset = OFFSET_TIFF_HEADER + offset_0th_ifd + 2;
    let mut offset_exif_ifd: Option<u32> = None;
    for i in 0..tag_num {
        if app1[(tag_offset + i*12)..(tag_offset + i*12 + 2)] == tag {
            let mut tmp = [0u8; 4];
            tmp.copy_from_slice(&app1[(tag_offset + i*12 + 8)..(tag_offset + i*12 + 12)]);
            offset_exif_ifd = Some(match byte_order {  // 起点はExif識別子の直後（TIFFヘッダの先頭）
                ByteOrder::BigEndian    => u32::from_be_bytes(tmp),
                ByteOrder::LittleEndian => u32::from_le_bytes(tmp),
            });
        }
    }
    if offset_exif_ifd.is_none() {
        return None;
    }
    let offset_exif_ifd = offset_exif_ifd.unwrap() as usize;

    // --- Exif IFDを読む ---
    let mut tmp = [0u8; 2];
    tmp.copy_from_slice(&app1[(OFFSET_TIFF_HEADER + offset_exif_ifd)..(OFFSET_TIFF_HEADER + offset_exif_ifd + 2)]);
    let tag_num = match byte_order {
        ByteOrder::BigEndian    => u16::from_be_bytes(tmp),
        ByteOrder::LittleEndian => u16::from_le_bytes(tmp),
    } as usize;

    // Date Time Originalタグを探す
    let tag = match byte_order {
        ByteOrder::BigEndian    => DATE_TIME_ORIGINAL.to_be_bytes(),
        ByteOrder::LittleEndian => DATE_TIME_ORIGINAL.to_le_bytes(),
    };
    let tag_offset = OFFSET_TIFF_HEADER + offset_exif_ifd + 2;
    let mut offset_date_time_original: Option<u32> = None;
    for i in 0..tag_num {
        if app1[(tag_offset + i*12)..(tag_offset + i*12 + 2)] == tag {
            // タイプとカウントは固定なので（それぞれASCII, 20）オフセットだけ読む。
            // ASCIIタイプには終端にNULL文字が付くので、意味をなす文字は19文字。
            let mut tmp = [0u8; 4];
            tmp.copy_from_slice(&app1[(tag_offset + i*12 + 8)..(tag_offset + i*12 + 12)]);
            offset_date_time_original = Some(match byte_order {  // 起点はExif識別子の直後（TIFFヘッダの先頭）
                ByteOrder::BigEndian    => u32::from_be_bytes(tmp),
                ByteOrder::LittleEndian => u32::from_le_bytes(tmp),
            });
        }
    }
    if offset_date_time_original.is_none() {
        return None;
    }
    let offset_date_time_original = offset_date_time_original.unwrap() as usize;

    let mut date_time_original = [0u8; 19];
    date_time_original.copy_from_slice(&app1[(OFFSET_TIFF_HEADER + offset_date_time_original)..(OFFSET_TIFF_HEADER + offset_date_time_original + 19)]);

    Some(date_time_original)
}

// EnumでDeg90とかを返したほうがいい
/// 画像の回転情報を読み込んで返す
pub fn get_rotate() -> u8 {
    0
}