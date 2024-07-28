

#[derive(Debug, clap::Parser)]
#[clap(author, version, about="PS3dec Remake is a remake of the original PS3 DISC decryption tool in rust", long_about = "PS3Dec is a tool to decrypt PS3 Redump ISOs Either use it as a CLI tool or Drag and drop the ISO on the executable to have it automatically decrypted provided the keys folder has the key required keys.")]
pub struct Ps3decargs {
    #[clap(short, long, help = "The path to the PS3 ISO file to decrypt.")]
    pub iso: String,

    #[clap(short, long, help = "The decryption key in Hexadecimal format of base-16.")]
    pub dk: Option<String>,

    #[clap(short, long, help = "Thread count, be careful this might vary from computer to computer.",default_value = "32")]
    pub tc: usize,

    #[clap(short, long, help = "Autodetect the right key for the iso based on its name then decrypt", action = clap::ArgAction::SetTrue)]
    pub auto: bool,

    #[clap(short,long,help = "Skip exit confirmation.",action = clap::ArgAction::SetTrue)]
    pub skip: bool,
}
