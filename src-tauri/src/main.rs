// リリースビルド時に Windows でコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    koemoji_lib::run()
}
