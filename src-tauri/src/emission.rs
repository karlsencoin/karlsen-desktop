//! Karlsen mainnet block reward schedule.
//!
//! Mirrors `consensus/src/processes/coinbase.rs` and the mainnet `Params` in rusty-karlsen
//! (karlsend 3.3.0): 1 block per second, Crescendo not activated on mainnet, 50 KLS per block
//! in the pre-deflationary phase, then a monthly reward table. Keep in sync with the node if
//! mainnet consensus parameters ever change.

use serde::Serialize;

/// Seconds per subsidy month: 30.4375 * 24 * 60 * 60.
const SECONDS_PER_MONTH: u64 = 2_629_800;
/// Mainnet DAA score at which the deflationary phase starts.
pub const DEFLATIONARY_PHASE_DAA_SCORE: u64 = 15_778_800 - 259_200;
/// Mainnet block subsidy before the deflationary phase, in sompi (50 KLS).
const PRE_DEFLATIONARY_PHASE_BASE_SUBSIDY: u64 = 5_000_000_000;
/// Mainnet blocks per second; the DAA score advances by this many units per second.
const MAINNET_BPS: u64 = 1;

/// Reward schedule summary for one DAA score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRewardInfo {
    /// Block subsidy at the given DAA score, in sompi (fees not included).
    pub current_reward_sompi: u64,
    /// Block subsidy after the next reduction, in sompi. Equal to the current one once emission has ended.
    pub next_reward_sompi: u64,
    /// DAA score at which the next reduction takes effect; `None` once emission has ended.
    pub next_reduction_daa_score: Option<u64>,
    /// Seconds until the next reduction (DAA units / BPS); `None` once emission has ended.
    pub seconds_until_next_reduction: Option<u64>,
}

fn subsidy_month(daa_score: u64) -> usize {
    ((daa_score - DEFLATIONARY_PHASE_DAA_SCORE) / MAINNET_BPS / SECONDS_PER_MONTH) as usize
}

fn subsidy_for_month(month: usize) -> u64 {
    SUBSIDY_BY_MONTH_TABLE[month.min(SUBSIDY_BY_MONTH_TABLE.len() - 1)].div_ceil(MAINNET_BPS)
}

/// Block subsidy in sompi at `daa_score`, exactly as karlsend computes it on mainnet.
pub fn block_subsidy(daa_score: u64) -> u64 {
    if daa_score < DEFLATIONARY_PHASE_DAA_SCORE {
        return PRE_DEFLATIONARY_PHASE_BASE_SUBSIDY;
    }
    subsidy_for_month(subsidy_month(daa_score))
}

/// Current reward, the next reward and when the next reduction happens.
pub fn block_reward_info(daa_score: u64) -> BlockRewardInfo {
    let current = block_subsidy(daa_score);
    let next_month = if daa_score < DEFLATIONARY_PHASE_DAA_SCORE { 0 } else { subsidy_month(daa_score) + 1 };
    let next_daa = DEFLATIONARY_PHASE_DAA_SCORE + next_month as u64 * SECONDS_PER_MONTH * MAINNET_BPS;
    if next_month >= SUBSIDY_BY_MONTH_TABLE.len() {
        return BlockRewardInfo {
            current_reward_sompi: current,
            next_reward_sompi: current,
            next_reduction_daa_score: None,
            seconds_until_next_reduction: None,
        };
    }
    BlockRewardInfo {
        current_reward_sompi: current,
        next_reward_sompi: block_subsidy(next_daa),
        next_reduction_daa_score: Some(next_daa),
        seconds_until_next_reduction: Some((next_daa - daa_score) / MAINNET_BPS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_deflationary_phase_pays_50_kls() {
        assert_eq!(block_subsidy(0), 5_000_000_000);
        assert_eq!(block_subsidy(DEFLATIONARY_PHASE_DAA_SCORE - 1), 5_000_000_000);
        let info = block_reward_info(DEFLATIONARY_PHASE_DAA_SCORE - 10);
        assert_eq!(info.next_reward_sompi, 4_400_000_000);
        assert_eq!(info.seconds_until_next_reduction, Some(10));
    }

    #[test]
    fn deflationary_phase_follows_monthly_table() {
        assert_eq!(block_subsidy(DEFLATIONARY_PHASE_DAA_SCORE), 4_400_000_000);
        assert_eq!(block_subsidy(DEFLATIONARY_PHASE_DAA_SCORE + SECONDS_PER_MONTH - 1), 4_400_000_000);
        assert_eq!(block_subsidy(DEFLATIONARY_PHASE_DAA_SCORE + SECONDS_PER_MONTH), 4_278_340_444);
    }

    #[test]
    fn mainnet_october_2026() {
        // DAA score seen on mainnet on 2026-10-03: month 29 of the deflationary phase.
        let info = block_reward_info(91_902_092);
        assert_eq!(info.current_reward_sompi, 1_951_236_555);
        assert_eq!(info.next_reward_sompi, 1_897_285_061);
        assert_eq!(info.next_reduction_daa_score, Some(DEFLATIONARY_PHASE_DAA_SCORE + 30 * SECONDS_PER_MONTH));
    }

    #[test]
    fn total_emission_matches_max_sompi() {
        // MAX_SOMPI in rusty-karlsen is 4_961_000_000 KLS; the schedule must stay below it.
        let total: u128 = PRE_DEFLATIONARY_PHASE_BASE_SUBSIDY as u128 * DEFLATIONARY_PHASE_DAA_SCORE as u128
            + SUBSIDY_BY_MONTH_TABLE.iter().map(|&s| s as u128 * SECONDS_PER_MONTH as u128).sum::<u128>();
        assert!(total <= 4_961_000_000u128 * 100_000_000);
        assert!(total > 4_960_000_000u128 * 100_000_000);
    }

    #[test]
    fn emission_end() {
        let end = DEFLATIONARY_PHASE_DAA_SCORE + SUBSIDY_BY_MONTH_TABLE.len() as u64 * SECONDS_PER_MONTH;
        let info = block_reward_info(end);
        assert_eq!(info.current_reward_sompi, 0);
        assert_eq!(info.next_reduction_daa_score, None);
    }
}

/// Monthly block subsidy table in sompi for 1 BPS, copied verbatim from rusty-karlsen
/// (`SUBSIDY_BY_MONTH_TABLE`, generated by `TestBuildSubsidyTable` in karlsend-go).
#[rustfmt::skip]
const SUBSIDY_BY_MONTH_TABLE: [u64; 793] = [
    4400000000, 4278340444, 4160044764, 4045019946, 3933175554, 3824423648, 3718678720, 3615857630, 3515879532, 3418665818,
    3324140054, 3232227917, 3142857142, 3055957460, 2971460545, 2889299962, 2809411110, 2731731177, 2656199086, 2582755450,
    2511342523, 2441904156, 2374385753, 2308734227, 2244897959, 2182826757, 2122471818, 2063785687, 2006722221, 1951236555,
    1897285061, 1844825321, 1793816087, 1744217254, 1695989823, 1649095876, 1603498542, 1559161969, 1516051298, 1474132633,
    1433373015, 1393740396, 1355203615, 1317732372, 1281297205, 1245869467, 1211421302, 1177925626, 1145356101, 1113687121,
    1082893784, 1052951881, 1023837868, 995528854, 968002582, 941237408, 915212289, 889906762, 865300930, 841375447,
    818111501, 795490800, 773495560, 752108486, 731312762, 711092039, 691430416, 672312434, 653723064, 635647687,
    618072093, 600982462, 584365357, 568207714, 552496829, 537220347, 522366259, 507922885, 493878868, 480223167,
    466945045, 454034062, 441480066, 429273187, 417403827, 405862653, 394640592, 383728819, 373118756, 362802060,
    352770620, 343016548, 333532175, 324310044, 315342904, 306623705, 298145590, 289901895, 281886137, 274092014,
    266513397, 259144329, 251979014, 245011820, 238237268, 231650031, 225244931, 219016932, 212961136, 207072782,
    201347240, 195780010, 190366712, 185103092, 179985010, 175008443, 170169477, 165464308, 160889237, 156440665,
    152115097, 147909130, 143819457, 139842864, 135976223, 132216494, 128560721, 125006030, 121549626, 118188791,
    114920883, 111743332, 108653640, 105649378, 102728184, 99887760, 97125873, 94440353, 91829086, 89290021,
    86821161, 84420565, 82086345, 79816666, 77609743, 75463841, 73377274, 71348400, 69375624, 67457395,
    65592204, 63778587, 62015115, 60300403, 58633103, 57011904, 55435531, 53902744, 52412338, 50963142,
    49554017, 48183853, 46851574, 45556133, 44296511, 43071717, 41880788, 40722788, 39596807, 38501960,
    37437384, 36402244, 35395726, 34417038, 33465410, 32540095, 31640365, 30765512, 29914848, 29087706,
    28283434, 27501400, 26740989, 26001603, 25282661, 24583598, 23903864, 23242925, 22600260, 21975365,
    21367749, 20776933, 20202453, 19643857, 19100706, 18572573, 18059044, 17559713, 17074189, 16602089,
    16143043, 15696689, 15262678, 14840666, 14430323, 14031326, 13643361, 13266124, 12899317, 12542652,
    12195849, 11858635, 11530745, 11211921, 10901912, 10600476, 10307373, 10022376, 9745258, 9475803,
    9213798, 8959037, 8711320, 8470453, 8236246, 8008515, 7787080, 7571768, 7362409, 7158840,
    6960898, 6768430, 6581284, 6399312, 6222372, 6050324, 5883033, 5720368, 5562200, 5408406,
    5258864, 5113457, 4972070, 4834593, 4700917, 4570937, 4444551, 4321660, 4202166, 4085977,
    3973000, 3863147, 3756331, 3652469, 3551479, 3453280, 3357798, 3264955, 3174679, 3086900,
    3001547, 2918555, 2837857, 2759390, 2683094, 2608906, 2536770, 2466629, 2398427, 2332110,
    2267628, 2204928, 2143962, 2084682, 2027040, 1970993, 1916495, 1863504, 1811979, 1761878,
    1713162, 1665793, 1619734, 1574949, 1531401, 1489058, 1447886, 1407852, 1368925, 1331074,
    1294270, 1258484, 1223687, 1189852, 1156953, 1124963, 1093858, 1063613, 1034204, 1005608,
    977803, 950767, 924479, 898917, 874062, 849894, 826395, 803545, 781327, 759723,
    738717, 718292, 698431, 679119, 660342, 642083, 624330, 607067, 590282, 573961,
    558091, 542659, 527655, 513065, 498879, 485085, 471673, 458631, 445950, 433619,
    421630, 409972, 398636, 387614, 376896, 366475, 356342, 346489, 336909, 327593,
    318535, 309728, 301164, 292837, 284740, 276867, 269211, 261768, 254530, 247492,
    240649, 233995, 227525, 221234, 215117, 209169, 203385, 197762, 192294, 186977,
    181807, 176780, 171892, 167139, 162518, 158024, 153655, 149406, 145275, 141258,
    137353, 133555, 129862, 126271, 122780, 119385, 116084, 112874, 109753, 106719,
    103768, 100899, 98109, 95396, 92758, 90194, 87700, 85275, 82917, 80624,
    78395, 76227, 74120, 72070, 70078, 68140, 66256, 64424, 62643, 60910,
    59226, 57589, 55996, 54448, 52943, 51479, 50055, 48671, 47325, 46017,
    44745, 43507, 42304, 41135, 39997, 38891, 37816, 36770, 35754, 34765,
    33804, 32869, 31960, 31077, 30217, 29382, 28569, 27779, 27011, 26264,
    25538, 24832, 24145, 23478, 22829, 22197, 21584, 20987, 20407, 19842,
    19294, 18760, 18241, 17737, 17247, 16770, 16306, 15855, 15417, 14990,
    14576, 14173, 13781, 13400, 13029, 12669, 12319, 11978, 11647, 11325,
    11012, 10707, 10411, 10123, 9843, 9571, 9307, 9049, 8799, 8556,
    8319, 8089, 7865, 7648, 7436, 7231, 7031, 6836, 6647, 6464,
    6285, 6111, 5942, 5778, 5618, 5463, 5312, 5165, 5022, 4883,
    4748, 4617, 4489, 4365, 4244, 4127, 4013, 3902, 3794, 3689,
    3587, 3488, 3391, 3298, 3206, 3118, 3031, 2948, 2866, 2787,
    2710, 2635, 2562, 2491, 2422, 2355, 2290, 2227, 2165, 2105,
    2047, 1990, 1935, 1882, 1830, 1779, 1730, 1682, 1636, 1590,
    1546, 1504, 1462, 1422, 1382, 1344, 1307, 1271, 1236, 1201,
    1168, 1136, 1104, 1074, 1044, 1015, 987, 960, 933, 908,
    882, 858, 834, 811, 789, 767, 746, 725, 705, 685,
    667, 648, 630, 613, 596, 579, 563, 548, 532, 518,
    503, 489, 476, 463, 450, 438, 425, 414, 402, 391,
    380, 370, 359, 349, 340, 330, 321, 312, 304, 295,
    287, 279, 271, 264, 257, 249, 243, 236, 229, 223,
    217, 211, 205, 199, 194, 188, 183, 178, 173, 168,
    164, 159, 155, 150, 146, 142, 138, 134, 131, 127,
    124, 120, 117, 114, 110, 107, 104, 101, 99, 96,
    93, 91, 88, 86, 83, 81, 79, 76, 74, 72,
    70, 68, 66, 65, 63, 61, 59, 58, 56, 54,
    53, 52, 50, 49, 47, 46, 45, 43, 42, 41,
    40, 39, 38, 37, 36, 35, 34, 33, 32, 31,
    30, 29, 28, 28, 27, 26, 25, 25, 24, 23,
    23, 22, 21, 21, 20, 20, 19, 18, 18, 17,
    17, 16, 16, 16, 15, 15, 14, 14, 13, 13,
    13, 12, 12, 12, 11, 11, 11, 10, 10, 10,
    9, 9, 9, 9, 8, 8, 8, 8, 7, 7,
    7, 7, 7, 6, 6, 6, 6, 6, 6, 5,
    5, 5, 5, 5, 5, 4, 4, 4, 4, 4,
    4, 4, 4, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 0,
];
