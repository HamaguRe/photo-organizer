//! 指定したフォルダに含まれる画像のファイル名を "撮影日時 + ハッシュ値" にする．
//! 日時情報が得られなかった場合は全部ハッシュ値．
//! ハッシュ値はCRC32の16進数表記なので常に8桁．
//!
//! 例えば，2023年1月23日の14時30分に撮影した場合は
//! 2023-01-23_1430_206cc7d9.jpg みたいになる．

use std::ffi::OsString;
use std::fs;
use std::fs::File;
use std::path;
use std::io::{self, Write, BufReader};

use clap::Parser;
use exif;
use crc32fast;
use rfd::FileDialog;
use rusttype::{Font, Scale};
use image::Rgba;

#[derive(Parser)]
struct Args {
    /// Print the date on the image (format: YYYY-MM-DD).
    #[arg(short, long)]
    date: bool,

    /// Recursive processing when subdirectories exist.
    #[arg(short, long)]
    recursion: bool,
}

fn main() {
    // コマンドライン引数を読む
    // -d or --date     : 日付印字
    // -r or --recursion: サブディレクトリを含めた再帰処理
    let args = Args::parse();

    // 処理するディレクトリを選択
    let dir_path = FileDialog::new()
        .set_directory("~/Pictures/")
        .pick_folder();

    if dir_path.is_none() {
        println!("Path is None.");
        std::process::exit(1);
    }

    println!("--- Info ---");
    println!("Change names of files in this directory: {}", dir_path.as_ref().unwrap().display());
    if args.date {
        println!("And, since you specified the -d option, I'll print the date on the image.");
    }
    println!("------------");

    // 実行確認
    let mut input = String::with_capacity(8);
    loop {
        print!("Can I start the process? [y/n]: ");
        io::stdout().flush().unwrap(); // 上記出力を強制フラッシュ
        io::stdin().read_line(&mut input).expect("Input error.");

        if input.starts_with('y') {
            break;
        } else if input.starts_with('n') {
            println!("Pushed 'n' key... program exit.");
            std::process::exit(0);
        } else {
            println!("Please push the key, 'y' or 'n'.");
        }
        input.clear();
    }

    println!("Processing...");
    change_names(&dir_path.unwrap(), args.date, args.recursion).unwrap();
    println!("Finish!")
}

// フルスクラッチでExifの読み出しと書き込みを実装したほうが綺麗にまとまりそう
// DateTimeOriginalしか読み出さないし。
//
// exif情報が吹っ飛んでしまう
// exifの回転を考慮していないので画像によっては回転した状態で保存されてしまう。
fn print_date(file_path: &path::PathBuf, date_txt: &str) {
    let mut img = image::open(file_path).unwrap();

    let font = include_bytes!("../fonts-DSEG_v046/DSEG7-Classic-MINI/DSEG7ClassicMini-Bold.ttf");
    let font = Font::try_from_bytes(font).expect("Could not read font data.");
    
    // 文字サイズが画像短辺の1/45になるようにする．
    let font_size = (img.width().min( img.height() ) as f32 / 45.0).round();

    let pos_x = img.width()  as i32 - font_size as i32 * 10;
    let pos_y = img.height() as i32 - font_size as i32 * 2;

    let scale = Scale::uniform(font_size);
    let color = Rgba([255, 130, 0, 255]);
    imageproc::drawing::draw_text_mut(&mut img, color, pos_x, pos_y, scale, &font, date_txt);

    img.save(file_path).expect("Failed to overwrite the file.");
}

/// 日付と時刻データを以下の文字列形式で返す。
/// 
/// YYYY-MM-DD_HHMM
fn get_date_time(reader: &mut BufReader<File>) -> Option<String>{
    // EXIF情報を取得
    let mut date_time = None;
    if let Ok(exif_data) = exif::Reader::new().read_from_container(reader) {
        // DateTimeOriginal タグを指定して値を取得
        if let Some(datetime_entry) = exif_data.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
            // タグが見つかった場合、タグの値を取得
            if let exif::Value::Ascii(ref values) = datetime_entry.value {
                let mut tag = values[0].to_vec();  // ASCII形式のバイト列（YYYY:MM:DD HH:MM:SS）
                
                // 文字列にしてしまうと弄りにくいので、バイト列の状態でフォーマットを整える
                tag[4]  = b'-';
                tag[7]  = b'-';
                tag[10] = b'_';
                tag[13] = tag[14];  // 一文字ずらして時刻のコロンを消す
                tag[14] = tag[15];
                tag.drain((tag.len() - 4)..);  // 末尾4文字（M:SS）を削除
                date_time = Some( String::from_utf8(tag).unwrap() );  // Some("YYYY-MM-DD_HHMM")
            }
        }        
    }
    date_time
}

/// 指定されたディレクトリ内の画像ファイル（jpg, png）のファイル名を書き換える．
/// 拡張子は小文字に統一される（JPG -> jpg, PNG -> png）
fn change_names(dir_path: &path::PathBuf, flag_print_date: bool, flag_sub_dir: bool) -> io::Result<()> {
    let mut images_cnt: usize = 0;
    for entry in fs::read_dir(dir_path)? {
        let file_path = entry?.path();
        if file_path.is_dir() {
            // サブフォルダを処理する場合は再帰処理
            if flag_sub_dir {
                change_names(&file_path, flag_print_date, true)?;
            }
            // スキップ（サブフォルダを処理し終わったら次に行く）
            continue;
        }

        // 拡張子を確認（jpg, png）
        let ext = file_path.extension().unwrap().to_ascii_lowercase();  // 小文字に変換
        if ext != OsString::from("jpg") && ext != OsString::from("png") {
            continue;  // jpgとpng以外は飛ばす
        } else {
            images_cnt += 1;
        }

        let date_time;
        let hash_crc32;
        {
            // ファイルを開く
            let file = File::open(&file_path).expect("File could not be opened.");
            let mut reader = BufReader::new(file);

            date_time = get_date_time(&mut reader);
            hash_crc32 = format!("{:x}", crc32fast::hash(reader.buffer()));
        }

        // 新しいファイル名を決定
        let mut new_file_name = String::with_capacity(32);
        if date_time.is_some() {
            new_file_name.push_str(&date_time.as_ref().unwrap());
            new_file_name.push('_');

            // 日付を印字
            if flag_print_date {
                print_date(&file_path, &date_time.unwrap()[0..10]);
            }
        }
        new_file_name.push_str(&hash_crc32);
        new_file_name.push('.');
        new_file_name.push_str(&ext.into_string().unwrap());

        // 新しいパスを作って書き換え
        let new_file_path = file_path.parent().unwrap().join(new_file_name);
        fs::rename(file_path, new_file_path)?;
    }
    println!("Number of images: {}", images_cnt);

    Ok(())
}