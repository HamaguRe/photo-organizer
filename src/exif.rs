//! Exifデータの読み出し・修正を行うためのモジュール
//! JPEGのみ

enum ByteOrder {
    BigEndian,
    LittleEndian,
}

/// APP1セグメント内におけるTIFFヘッダの開始オフセット
const OFFSET_TIFF_HEADER: usize = 10;

// タグ番号
const ORIENTATION: u16 = 0x0112;
const EXIF_IFD_POINTER: u16 = 0x8769;
const DATE_TIME_ORIGINAL: u16 = 0x9003;

/// 2byteのスライスをu16として復号する．
/// 
/// slice.len() == 2とすること（slice.len() != 2の場合にはpanic）．
fn decode_u16(slice: &[u8], byte_order: &ByteOrder) -> u16 {
    let mut tmp = [0u8; 2];
    tmp.copy_from_slice(slice);

    match byte_order {
        ByteOrder::BigEndian    => u16::from_be_bytes(tmp),
        ByteOrder::LittleEndian => u16::from_le_bytes(tmp),
    }
}

/// 4byteのスライスをu32として復号する．
/// 
/// slice.len() == 4とすること（slice.len() != 4の場合にはpanic）．
fn decode_u32(slice: &[u8], byte_order: &ByteOrder) -> u32 {
    let mut tmp = [0u8; 4];
    tmp.copy_from_slice(slice);

    match byte_order {
        ByteOrder::BigEndian    => u32::from_be_bytes(tmp),
        ByteOrder::LittleEndian => u32::from_le_bytes(tmp),
    }
}

/// 回転情報を消した（回転なしの状態にした）APP1セグメントを返す．
pub fn clear_orientation(jpeg_binary: &[u8]) -> Vec<u8> {
    let ref_app1 = get_app1(jpeg_binary).unwrap();
    let mut app1 = vec![0; ref_app1.len()];
    app1.copy_from_slice(ref_app1);

    let byte_order = if app1[OFFSET_TIFF_HEADER..(OFFSET_TIFF_HEADER + 2)] == [0x4D, 0x4D] {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian
    };

    // 0th IFDのオフセットを読む．起点はTIFFヘッダの先頭（Exif識別子の直後）．
    let offset_0th_ifd = decode_u32(&app1[(OFFSET_TIFF_HEADER + 4)..(OFFSET_TIFF_HEADER + 8)], &byte_order) as usize;

    // Orientationを読む
    let orientation_slice = read_tag(&app1, offset_0th_ifd, ORIENTATION, &byte_order);
    if orientation_slice.is_some() {
        // スライスが元の配列のどこの部分であるかを逆算して，orientationタグのvalueを書き直す．
        let app1_ptr = app1.as_ptr();
        let orientation_ptr = orientation_slice.unwrap().as_ptr();

        // APP1セグメント内におけるOrientationタグのvalueの開始オフセット
        let orientation_offset = orientation_ptr as usize - app1_ptr as usize;
        let tmp = match byte_order {  // 1（回転なし）を書き込む
            ByteOrder::BigEndian => 1_u16.to_be_bytes(),
            ByteOrder::LittleEndian => 1_u16.to_le_bytes(),
        };
        app1[orientation_offset] = tmp[0];
        app1[orientation_offset + 1] = tmp[1];
    }
    app1
}

/// APP0セグメントの次のセグメントの先頭のインデックスを返す．
pub fn next_app0_index(non_app1_binary: &[u8]) -> Result<usize, &'static str> {
    // JPEG画像先頭のSOIマーカを確認
    if non_app1_binary[..2] != [0xFF, 0xD8] {
        return Err("SOI marker does not exist.");
    }

    // APP0セグメントの終端を探す
    // APP0セグメントがない場合はSOIマーカの次のインデックスを返す．
    let mut next_app0 = 2;  // APP0の次のセグメント先頭を指すインデックス
    for i in 2..(non_app1_binary.len() - 1) {
        if non_app1_binary[i..=(i + 1)] == [0xFF, 0xE0] {  // APP0のマーカを探す
            // セグメント長は必ずビッグエンディアン
            let segment_len = decode_u16(&non_app1_binary[(i+2)..(i+4)], &ByteOrder::BigEndian) as usize;
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
pub fn get_app1(jpeg_binary: &[u8]) -> Option<&[u8]> {
    for i in 0..(jpeg_binary.len() - 1) {
        if jpeg_binary[i..=(i + 1)] == [0xFF, 0xE1] {  // APP1のマーカを探す
            // セグメント長は必ずビッグエンディアン
            let segment_len = decode_u16(&jpeg_binary[(i+2)..(i+4)], &ByteOrder::BigEndian) as usize;
            // Exif識別子を確認（XMPの可能性があるため）
            if &jpeg_binary[(i+4)..(i+9)] == b"Exif\0" {
                return Some(&jpeg_binary[i..(i + segment_len + 2)]);
            }
        }
    }
    None
}

/// 指定したタグのvalueが書かれた領域をスライスで返す．
/// 
/// * ifd_offset: タグを読み出したいIFDの開始オフセット（起点はTIFFヘッダの先頭）
/// * tag: タグ番号
/// * byte_order: TIFFヘッダに書かれているバイトオーダー
fn read_tag<'a>(app1: &'a [u8], ifd_offset: usize, tag: u16, byte_order: &ByteOrder) -> Option<&'a [u8]> {
    // タグ数を読む
    let tmp = OFFSET_TIFF_HEADER + ifd_offset;
    let tag_num = decode_u16(&app1[tmp..(tmp + 2)], byte_order) as usize;
    
    let tag = match byte_order {
        ByteOrder::BigEndian    => tag.to_be_bytes(),
        ByteOrder::LittleEndian => tag.to_le_bytes(),
    };

    let mut tag_field_offset = tmp + 2;  // タグフィールドの開始オフセット
    for _ in 0..tag_num {
        if app1[tag_field_offset..(tag_field_offset + 2)] == tag {  // タグをチェック
            // valueのタイプを確認（SHORTかASCIIか...とか）
            let value_type = decode_u16(&app1[(tag_field_offset + 2)..(tag_field_offset + 4)], byte_order);
            
            // valueのカウントを確認
            let count = decode_u32(&app1[(tag_field_offset + 4)..(tag_field_offset + 8)], byte_order) as usize;

            // valueを表現するのに必要なデータ長を計算する
            let value_bytes = match value_type {
                2 => 1,  // ASCII（1文字1byte）
                3 => 2,  // SHORT (16bit符号無し整数)
                4 => 4,  // LONG （32bit符号無し整数）
                _ => return None
            } * count;

            if value_bytes <= 4 {
                // 4byte以下のデータはオフセット領域に直書きされている（左詰め）
                return Some( &app1[(tag_field_offset + 8)..(tag_field_offset + 8 + value_bytes)] );
            } else {
                // valueのオフセットを調べる（起点はTIFFヘッダの先頭）
                let value_offset = decode_u32(&app1[(tag_field_offset + 8)..(tag_field_offset + 12)], byte_order) as usize;

                return Some( &app1[(OFFSET_TIFF_HEADER + value_offset)..(OFFSET_TIFF_HEADER + value_offset + value_bytes)] );
            }
        }
        tag_field_offset += 12;  // 次のタグフィールドへ
    }

    None
}

/// DateTimeOriginalタグのvalueを返す（ASCII文字列で，終端のNULL文字は除く）．
/// 
/// Format: YYYY:MM:DD HH:MM:SS (Example: 2015:09:27 11:43:11)
pub fn get_date_time_original(jpeg_binary: &[u8]) -> Option<[u8; 19]> {
    let app1 = get_app1(jpeg_binary)?;

    let byte_order = if app1[OFFSET_TIFF_HEADER..(OFFSET_TIFF_HEADER + 2)] == [0x4D, 0x4D] {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian  // [0x49, 0x49]ならリトルエンディアン
    };

    // 0th IFDのオフセットを読む．起点はTIFFヘッダの先頭（Exif識別子の直後）．
    let offset_0th_ifd = decode_u32(&app1[(OFFSET_TIFF_HEADER + 4)..(OFFSET_TIFF_HEADER + 8)], &byte_order) as usize;

    // Exif IFDの開始オフセットを読む．起点はTIFFヘッダの先頭．
    let tmp = read_tag(app1, offset_0th_ifd, EXIF_IFD_POINTER, &byte_order)?;
    let offset_exif_ifd = decode_u32(tmp, &byte_order);

    // Exif IFDのDateTimeOriginalタグを読む
    let tmp = read_tag(app1, offset_exif_ifd as usize, DATE_TIME_ORIGINAL, &byte_order)?;
    let mut date_time_original = [0u8; 19];
    date_time_original.copy_from_slice(&tmp[..19]);  // ASCIIの場合にはバイトオーダーは気にしなくていいっぽい

    Some(date_time_original)
}

/// 画像の回転情報を読み込んで返す
pub fn get_orientation(jpeg_binary: &[u8]) -> Option<u16> {
    let app1 = get_app1(jpeg_binary)?;

    let byte_order = if app1[OFFSET_TIFF_HEADER..(OFFSET_TIFF_HEADER + 2)] == [0x4D, 0x4D] {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian
    };

    // 0th IFDのオフセットを読む．起点はTIFFヘッダの先頭（Exif識別子の直後）．
    let offset_0th_ifd = decode_u32(&app1[(OFFSET_TIFF_HEADER + 4)..(OFFSET_TIFF_HEADER + 8)], &byte_order) as usize;

    // Orientationを読む
    let tmp = read_tag(app1, offset_0th_ifd, ORIENTATION, &byte_order)?;
    let orientation = decode_u16(tmp, &byte_order);

    // orientationは1〜8の値をとる
    if orientation == 0 || orientation > 8 {
        None
    } else {
        Some(orientation)
    }
}
