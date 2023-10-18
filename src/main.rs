//! 指定したフォルダに含まれる画像のファイル名を "撮影日時 + ハッシュ値" にする．
//! 日時情報が得られなかった場合は全部ハッシュ値．
//! ハッシュ値はCRC32の16進数表記なので常に8桁．
//!
//! 例えば，2023年1月23日の14時30分に撮影した場合は
//! 2023-01-23_1430_206cc7d9.jpg みたいになる．

// $ RUSTFLAGS='-C target-cpu=native -C opt-level=3' cargo build --release

use std::ffi::OsString;
use std::fs;
use std::path;
use std::io::{self, Write};

use clap::Parser;
use crc32fast;
use rfd::FileDialog;
use rusttype::{Font, Scale};
use image::Rgba;
use imageproc::drawing;

mod exif;

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
        println!("Note that it will overwrite existing image data!!");
    }
    if args.recursion {
        println!("The -r option was specified. Subdirectories are also included in the process.");
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
    match change_names(&dir_path.unwrap(), args.date, args.recursion) {
        Ok(()) => println!("Finish!"),
        Err(e) => println!("Error: {}", e),
    }
}

// exif情報が吹っ飛んでしまう
// exifの回転を考慮していないので画像によっては回転した状態で保存されてしまう。
fn print_date(file_path: &path::PathBuf, jpeg_binary: &[u8], date_txt: &str) {
    {
        let font = include_bytes!("../fonts-DSEG_v046/DSEG7-Classic-MINI/DSEG7ClassicMini-Bold.ttf");
        let font = Font::try_from_bytes(font).expect("Could not read font data.");

        let mut img = image::load_from_memory(jpeg_binary).unwrap();

        // Exif情報を読んで画像を回す
        
        // 文字サイズが画像短辺の1/45になるようにする．
        let font_size = (img.width().min( img.height() ) as f32 / 45.0).round();
    
        let pos_x = img.width()  as i32 - font_size as i32 * 10;
        let pos_y = img.height() as i32 - font_size as i32 * 2;
    
        let scale = Scale::uniform(font_size);
        let color = Rgba([255, 130, 0, 255]);
        drawing::draw_text_mut(&mut img, color, pos_x, pos_y, scale, &font, date_txt);
    
        // 品質を指定して保存したい
        img.save(file_path).expect("Failed to overwrite the file.");
    }

    // APP0セグメント内の回転情報を直す

    let non_app1_binary = std::fs::read(&file_path).expect("Failed to load image file.");
    let mut w = io::BufWriter::new(fs::File::create(file_path).unwrap());
    let next_app0 = exif::next_app0_index(&non_app1_binary).unwrap();
    w.write(&non_app1_binary[..next_app0]).unwrap();  // 先頭からAPP0の終わりまで書き込む
    w.write(exif::get_app1(jpeg_binary).unwrap()).unwrap(); // APP1セグメント挿入
    w.write(&non_app1_binary[next_app0..]).unwrap();  // 残りを書き込む
    w.flush().expect("File overwrite failed.");
}

/// 日付と時刻データを以下の文字列形式で返す。
/// 
/// YYYY-MM-DD_HHMM
fn get_date_time(jpeg_binary: &[u8]) -> Option<String> {
    let mut val = exif::get_date_time_original(exif::get_app1(&jpeg_binary)?)?;

    // 文字列にしてしまうと弄りにくいので、バイト列の状態でフォーマットを整える
    val[4]  = b'-';
    val[7]  = b'-';
    val[10] = b'_';
    val[13] = val[14];  // 一文字ずらして時刻のコロンを消す
    val[14] = val[15];

    Some( String::from_utf8(val[..15].to_vec()).unwrap() )
}

// PNGからの日付情報の読み出しにはまだ未対応（そもそもPNGには日時情報を保持する仕組みがない？）
// 
/// 指定されたディレクトリ内の画像ファイルのファイル名を書き換える．
/// 拡張子は小文字に統一される．
fn change_names(dir_path: &path::PathBuf, flag_print_date: bool, flag_sub_dir: bool) -> io::Result<()> {
    for entry in fs::read_dir(dir_path)? {  // ディレクトリ内要素のループ
        let file_path = entry?.path();
        if file_path.is_dir() {
            // サブフォルダを処理する場合は再帰処理
            if flag_sub_dir {
                change_names(&file_path, flag_print_date, true)?;
            }
            // スキップ（サブフォルダを処理し終わったら次に行く）
            continue;
        }

        // 拡張子を確認
        let ext = match file_path.extension() {
            Some(ext) => ext.to_ascii_lowercase(),  // 小文字に変換
            None => continue,
        };
        if ext != OsString::from("jpg") {
            continue;  // jpg以外は飛ばす
        }

        // 画像データ読み込み
        let jpeg_binary = fs::read(&file_path).expect("Failed to load image file.");

        let date_time = get_date_time(&jpeg_binary);  // 現状JPEGしか処理できない
        let hash_crc32 = format!("{:x}", crc32fast::hash(&jpeg_binary));

        // 新しいファイル名を決定
        let mut new_file_name = String::with_capacity(32);
        if date_time.is_some() {
            new_file_name.push_str(&date_time.as_ref().unwrap());
            new_file_name.push('_');

            // 日付を印字
            if flag_print_date {
                print_date(&file_path, &jpeg_binary, &date_time.unwrap()[..10]);
            }
        }
        new_file_name.push_str(&hash_crc32);
        new_file_name.push('.');
        new_file_name.push_str(ext.to_str().unwrap());

        // 新しいパスを作って書き換え
        let new_file_path = file_path.parent().unwrap().join(new_file_name);
        fs::rename(file_path, new_file_path)?;
    }

    Ok(())
}