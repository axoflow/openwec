use anyhow::{bail, Context, Result};
use bitreader::BitReader;
use log::debug;

const CTRLSYMB_FLUSH: u16 = 0b1111111110000;
const CTRLSYMB_SCHEME_1: u16 = 0b1111111110001;
const CTRLSYMB_SCHEME_2: u16 = 0b1111111110010;
const CTRLSYMB_FILE_MARK: u16 = 0b1111111110011;
const CTRLSYMB_END_OF_RECORD: u16 = 0b1111111110100;
const CTRLSYMB_RESET_1: u16 = 0b1111111110101;
const CTRLSYMB_RESET_2: u16 = 0b1111111110110;
const CTRLSYMB_END_MARKER: u16 = 0b1111111111111;

pub fn decompress(compressed_bytes: &Vec<u8>) -> Result<Vec<u8>> {
    // Implemented according to ECMA-321
    debug!(
        "Try to decompress SLDC data ({} compressed bytes)",
        compressed_bytes.len()
    );

    let mut reader = BitReader::new(compressed_bytes);
    let mut res: Vec<u8> = Vec::new();

    let mut scheme_1 = false;
    let mut scheme_2 = false;
    let mut history_buffer: Vec<u8> = vec![0; 1024];
    let mut history_index: usize = 0;
    while reader.remaining() > 0 {
        // Try to find a control symbol
        if reader.peek_u16(9)? == 0x1FF {
            // 0b111111111
            let symb = reader.read_u16(13)?;
            match symb {
                CTRLSYMB_FLUSH => (),
                CTRLSYMB_SCHEME_1 => scheme_1 = true,
                CTRLSYMB_SCHEME_2 => scheme_2 = true,
                CTRLSYMB_FILE_MARK => (),
                CTRLSYMB_END_OF_RECORD => {
                    debug!(
                        "SLDC decompression succeed ({} uncompressed bytes)",
                        res.len()
                    );
                    return Ok(res);
                }
                CTRLSYMB_RESET_1 => {
                    scheme_1 = true;
                    history_buffer = vec![0; 1024];
                }
                CTRLSYMB_RESET_2 => {
                    scheme_2 = true;
                    history_buffer = vec![0; 1024];
                }
                CTRLSYMB_END_MARKER => (),
                _ => bail!("Found invalid control symbol"),
            }
        } else if scheme_1 {
            let is_literal = !reader.read_bool()?;
            if is_literal {
                // This is a Literal 1 Data Symbol
                // The next 8 bits represent the data byte
                let c = reader.read_u8(8)?;
                // Store c in result buffer
                res.push(c);
                // Store c in rotating history buffer
                history_buffer[history_index] = c;
                history_index = (history_index + 1) % 1024;
            } else {
                // This a Copy Pointer Data Symbol
                // We need to read the MCF (Match Count Field) in order
                // to find the size of the Match String.
                let next_4_bits = reader.peek_u16(4)?;
                let size: u16 = if (next_4_bits & 0b1000) == 0 {
                    reader.skip(2)?;
                    // If the next bit is 0, then the MCF is `0b00` or `0b01`
                    // and the size is respectively `2` or `3`
                    2 + ((next_4_bits & 0b0100) >> 2)
                } else if (next_4_bits & 0b0100) == 0 {
                    // If the next first bits are 10, then the MCF is `0b10xy`
                    reader.skip(4)?;
                    4 + (next_4_bits & 0b0011)
                } else if (next_4_bits & 0b0010) == 0 {
                    // If the next first bits are 110, then the MCF is `0b110xyz`
                    reader.skip(3)?;
                    8 + reader.read_u16(3)?
                } else if (next_4_bits & 0b0001) == 0 {
                    // If the next first bits are 1110, then the MCF is `0b1110xyza`
                    reader.skip(4)?;
                    16 + reader.read_u16(4)?
                } else if (next_4_bits & 0b0001) == 1 {
                    // Check for reserved and control symbols
                    reader.skip(4)?;
                    let next = reader.read_u16(8)?;
                    if next & 0b11110000 == 0b11110000 {
                        bail!("Found a Reserved or a Control Symbol instead of a Copy Pointer Data Symbol");
                    }
                    // the MCF is `0b1111 abcdefgh`
                    32 + next
                } else {
                    bail!("Found invalid Match Count Field value");
                };

                let displacement_field = reader.read_u16(10)?;

                for k in displacement_field..displacement_field + size {
                    // Find c in history_buffer
                    let c = *history_buffer
                        .get((k % 1024) as usize)
                        .context("Index not found in history buffer")?;
                    // Store c in result buffer
                    res.push(c);
                    // Store c in rotating history buffer
                    history_buffer[history_index] = c;
                    history_index = (history_index + 1) % 1024;
                }
            }
        } else if scheme_2 {
            let c = reader.read_u8(8)?;
            if c == 0xff {
                reader.skip(1)?;
            }
            // Store c in result buffer
            res.push(c);
            // Store c in rotating history buffer
            history_buffer[history_index] = c;
            history_index = (history_index + 1) % 1024;
        } else {
            bail!("Could not uncompress data");
        }
    }

    bail!("Missing END_OF_RECORD control symbol !");
}

// We only need to decompress data. Therefore, compress function
// is not to be implemented for now.
// pub fn compress(bytes: &Vec<u8>) -> Vec<u8> {}

#[cfg(test)]
mod tests {
    use super::*;
    use hex::FromHex;

    #[test]
    fn decompress_test_string() -> Result<()> {
        let test_string_compressed = Vec::from_hex("ffb3a32b9ba1039ba3934b733ffd0000")?;
        let test_string = decompress(&test_string_compressed)?;

        assert_eq!(test_string, "test string".as_bytes());
        Ok(())
    }

    #[test]
    fn decompress_heartbeat() -> Result<()> {
        let heartbeat_compressed = Vec::from_hex("ffabfdfc3c001cc003a00114006e001d80065001b0006f001c281a20001e0006da079bb2019cc003d000880068001d00074a098e8002f000bc0077001de8722ea1e8ce87e6f001c80067a1b8c80030000c28862fa298d68ae73a239868622da0af602845dc1730f8c2739800c748f32d1fcc349d176c7de03072118ef4b3e0270d322b8e29a661001900064a5f99698e73001a690667f08f265f9891665c8da6ea84f98ea3bfc0d01325634e80033754c4c5962f49d618e3756bc63d5afc24473acaf8162387c0d7336d56cc75a137e5293357b45d60b37edadf863543e00680224589f06e196c1b8948c36572a58747de17a4e5553c0818570c99874c39a2c5d91c4c76953659ec6a001cd53702e748b8e6003ad8970fc1cad2e70cab26fcbba2f001080036b6d912dbe42cd522d0011adca30000dedd634a03916dce2db568d2c7246b7f916dde450010edd244b6e8dee2244b8ae14818d99f06d709b0796b71d0009b72bc3f58a60ca0024dc7fb3ac6df9cbcb8eab968069dba3ad60eb7cb3a000ab9a3713163d4c75864ceba66cd4673c8ee3edd1231b78fc0288c651e6ce384cc52a1d9c284a79e397e04620f5377e0de51d290675dfeff2c41b12f721934ae0bf3c29b29e783a8dd9b4cf34bf37e475e1947618302c6920f409463e367fcc06e32569ce97efe8667c0a2617ee593956cf0e2186bdfc91ee4ab614c3bec2d3c5c3cdd68532e41d385884a76073d58a7e24b00d6bafa3706197e7e0509705309bb4b4c35bde01e1d6cbb8f10759be487937c8d7191a5c24836691bdabf17b10c0033b67e2ce22db1f17f10adb635b76e2e81a5b746b713215b8f83f5e4a83508133883f88cb45f0eae738940cf9954cb74116d904a7f15f1c41707c64186608997d4c3f4dc9ca7bc3f504126fd3f12f2ba56439a738c73778ced8646cdd645b68e2e01a51bc83469c0d041a2ee07a1b5194859b648945b20d24f03a10b382f9804e00013f3459cb3caf02c089f1f0648b9b3f15f0d690e36a428ca91e43a4be0721cd45c619b846f527c2498bb88b9b4f16e6120c0d832e24ff98c2e8001c732e9baf3f8c6be2c0c0c7a93fe06238641e2598a95fd36d7d4df5db3a57a4a16031b54dc5db9aca7c671f813e43cb228eb2f2e1dec47066b2fe232e1d41bb2cc9972dc9a2cba74cba22fb139cebc268b3b9b6c6a73b0dea6c39d91cc3683175a8ce57b7c45073b588cb30f8d2d6e30a2c8bed2276e32ce3b6c39032ae58e762ee2e616dba48d6eb22652fc12037ca7236b7c8d2e0246b8290ae1e39ca3a37ca578c7b1137a252e0f5106d1a6b0014b8baf86c0cb72fd7fd8a287400245d74c3b6a71b2636a215aacc8003cf1d048bc6cb39ef63f371de0dfb5dc116f5b8ea8d97bfc4603e007fd0000e20b0000")?;
        let heartbeat = Vec::from_hex("fffe3c0073003a0045006e00760065006c006f0070006500200078006d006c006e0073003a0073003d00220068007400740070003a002f002f007700770077002e00770033002e006f00720067002f0032003000300033002f00300035002f0073006f00610070002d0065006e00760065006c006f00700065002200200078006d006c006e0073003a0061003d00220068007400740070003a002f002f0073006300680065006d00610073002e0078006d006c0073006f00610070002e006f00720067002f00770073002f0032003000300034002f00300038002f00610064006400720065007300730069006e0067002200200078006d006c006e0073003a0065003d00220068007400740070003a002f002f0073006300680065006d00610073002e0078006d006c0073006f00610070002e006f00720067002f00770073002f0032003000300034002f00300038002f006500760065006e00740069006e0067002200200078006d006c006e0073003a0077003d00220068007400740070003a002f002f0073006300680065006d00610073002e0064006d00740066002e006f00720067002f007700620065006d002f00770073006d0061006e002f0031002f00770073006d0061006e002e007800730064002200200078006d006c006e0073003a0070003d00220068007400740070003a002f002f0073006300680065006d00610073002e006d006900630072006f0073006f00660074002e0063006f006d002f007700620065006d002f00770073006d0061006e002f0031002f00770073006d0061006e002e0078007300640022003e003c0073003a004800650061006400650072003e003c0061003a0054006f003e0068007400740070003a002f002f007300720076002e00770069006e0064006f006d00610069006e002e006c006f00630061006c003a0035003900380035002f00770073006d0061006e002f0073007500620073006300720069007000740069006f006e0073002f00420036004200440042004200350039002d0046004200300037002d0034004500450035002d0038003400310046002d004500420045004300390044003600370043004400440034002f0031003c002f0061003a0054006f003e003c006d003a004d0061006300680069006e00650049004400200078006d006c006e0073003a006d003d00220068007400740070003a002f002f0073006300680065006d00610073002e006d006900630072006f0073006f00660074002e0063006f006d002f007700620065006d002f00770073006d0061006e002f0031002f006d0061006300680069006e006500690064002200200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c007300650022003e00770069006e00310030002e00770069006e0064006f006d00610069006e002e006c006f00630061006c003c002f006d003a004d0061006300680069006e006500490044003e003c0061003a005200650070006c00790054006f003e003c0061003a004100640064007200650073007300200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200740072007500650022003e0068007400740070003a002f002f0073006300680065006d00610073002e0078006d006c0073006f00610070002e006f00720067002f00770073002f0032003000300034002f00300038002f00610064006400720065007300730069006e0067002f0072006f006c0065002f0061006e006f006e0079006d006f00750073003c002f0061003a0041006400640072006500730073003e003c002f0061003a005200650070006c00790054006f003e003c0061003a0041006300740069006f006e00200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200740072007500650022003e0068007400740070003a002f002f0073006300680065006d00610073002e0064006d00740066002e006f00720067002f007700620065006d002f00770073006d0061006e002f0031002f00770073006d0061006e002f004800650061007200740062006500610074003c002f0061003a0041006300740069006f006e003e003c0077003a004d006100780045006e00760065006c006f0070006500530069007a006500200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200740072007500650022003e003500310032003000300030003c002f0077003a004d006100780045006e00760065006c006f0070006500530069007a0065003e003c0061003a004d00650073007300610067006500490044003e0075007500690064003a00450045004300300034004600370034002d0041003200370044002d0034004300330041002d0041004500460035002d004200430035004200460035003400330035003900420041003c002f0061003a004d00650073007300610067006500490044003e003c0077003a004c006f00630061006c006500200078006d006c003a006c0061006e0067003d00220065006e002d00550053002200200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c0073006500220020002f003e003c0070003a0044006100740061004c006f00630061006c006500200078006d006c003a006c0061006e0067003d00220065006e002d00550053002200200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c0073006500220020002f003e003c0070003a00530065007300730069006f006e0049006400200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c007300650022003e0075007500690064003a00390038003100430035003300300046002d0042004500320041002d0034004100410042002d0042004100430042002d003600460042003400430044003100410031003400410042003c002f0070003a00530065007300730069006f006e00490064003e003c0070003a004f007000650072006100740069006f006e0049004400200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c007300650022003e0075007500690064003a00450041003200450045003500360036002d0032004300430031002d0034003900410030002d0041003700320036002d004200430045003700440043003300350036004500320032003c002f0070003a004f007000650072006100740069006f006e00490044003e003c0070003a00530065007100750065006e006300650049006400200073003a006d0075007300740055006e006400650072007300740061006e0064003d002200660061006c007300650022003e0031003c002f0070003a00530065007100750065006e0063006500490064003e003c0077003a004f007000650072006100740069006f006e00540069006d0065006f00750074003e0050005400360030002e0030003000300053003c002f0077003a004f007000650072006100740069006f006e00540069006d0065006f00750074003e003c0065003a004900640065006e00740069006600690065007200200078006d006c006e0073003a0065003d00220068007400740070003a002f002f0073006300680065006d00610073002e0078006d006c0073006f00610070002e006f00720067002f00770073002f0032003000300034002f00300038002f006500760065006e00740069006e00670022003e00320031003900430035003300350033002d0035004600330044002d0034004300440037002d0041003600340034002d004600360042003600390045003500370043003100430031003c002f0065003a004900640065006e007400690066006900650072003e003c0077003a00410063006b005200650071007500650073007400650064002f003e003c002f0073003a004800650061006400650072003e003c0073003a0042006f00640079003e003c0077003a004500760065006e00740073003e003c002f0077003a004500760065006e00740073003e003c002f0073003a0042006f00640079003e003c002f0073003a0045006e00760065006c006f00700065003e00")?;

        assert_eq!(decompress(&heartbeat_compressed)?, heartbeat);
        Ok(())
    }
}
