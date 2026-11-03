# Full-wave backend vs CST and FDM (620 cases with results)

## FW-E vs CST-E

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.003 | 0.003 | 0.027 | -0.075 | anchor_forsten_suite_5 |
| anchor | eeff_even | 7 | -0.014 | 0.014 | 0.047 | -0.072 | anchor_forsten_suite_7 |
| anchor | eeff_odd | 7 | -0.019 | 0.019 | 0.064 | -0.073 | anchor_forsten_suite_7 |
| anchor | z0 | 16 | +0.074 | 0.074 | 0.134 | +0.190 | anchor_thunderscope_xs_6 |
| anchor | zdiff | 7 | +0.051 | 0.051 | 0.097 | +0.101 | anchor_thunderscope_xs_4 |
| anchor | zeven | 7 | +0.039 | 0.051 | 0.081 | +0.088 | anchor_forsten_suite_7 |
| board_other | eeff | 16 | -0.001 | 0.002 | 0.006 | +0.013 | board_vna_06c826e75e |
| board_other | z0 | 16 | +0.042 | 0.046 | 0.075 | +0.085 | board_vna_a7541acf6b |
| board_target | eeff | 88 | +0.002 | 0.004 | 0.010 | -0.019 | board_vna_79bf5540db |
| board_target | eeff_even | 32 | -0.001 | 0.003 | 0.011 | +0.015 | board_ts_54b3d93297 |
| board_target | eeff_odd | 32 | -0.013 | 0.013 | 0.024 | -0.033 | board_ts_169daecc6a |
| board_target | z0 | 88 | +0.019 | 0.021 | 0.073 | +0.120 | board_vna_79bf5540db |
| board_target | zdiff | 32 | +0.047 | 0.047 | 22.484 | +51.297 | board_ts_ea1df30155 |
| board_target | zeven | 32 | -0.002 | 0.031 | 1.419 | -14.526 | board_ts_ea1df30155 |
| broadside | eeff_even | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.1_off0_w0.2 |
| broadside | eeff_odd | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.1_off0_w0.2 |
| broadside | zdiff | 8 | +0.010 | 0.012 | 0.025 | +0.025 | bsl_hm0.1_off0_w0.1 |
| broadside | zeven | 8 | +0.038 | 0.038 | 0.050 | +0.051 | bsl_hm0.1_off0_w0.1 |
| cpw_noplane | eeff | 8 | -0.043 | 0.043 | 0.062 | -0.064 | cpw_w0.3_g0.4 |
| cpw_noplane | z0 | 8 | +0.077 | 0.077 | 0.094 | +0.096 | cpw_w0.3_g0.25 |
| cpwg | eeff | 30 | -0.020 | 0.021 | 0.070 | -0.126 | cpwg_u0.5_g0.25 |
| cpwg | z0 | 30 | +0.044 | 0.044 | 0.113 | +0.152 | cpwg_u1_g0.25 |
| diff_cpwg | eeff_even | 6 | -0.018 | 0.018 | 0.048 | -0.056 | dcpwg_s1_g0.5 |
| diff_cpwg | eeff_odd | 6 | -0.050 | 0.050 | 0.078 | -0.085 | dcpwg_s0.5_g0.5 |
| diff_cpwg | zdiff | 6 | +0.062 | 0.062 | 0.101 | +0.104 | dcpwg_s0.5_g0.5 |
| diff_cpwg | zeven | 6 | +0.045 | 0.045 | 0.077 | +0.082 | dcpwg_s1_g0.5 |
| diff_microstrip | eeff_even | 34 | -0.023 | 0.025 | 0.060 | -0.147 | dms_u0.5_s4 |
| diff_microstrip | eeff_odd | 34 | -0.043 | 0.046 | 0.106 | -0.167 | dms_u0.5_s0.25 |
| diff_microstrip | zdiff | 34 | +0.062 | 0.062 | 0.116 | +0.165 | dms_u0.5_s0.25 |
| diff_microstrip | zeven | 34 | +0.043 | 0.043 | 0.080 | +0.161 | dms_u0.5_s4 |
| diff_stripline | eeff_even | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u0.5_s0.25_r3 |
| diff_stripline | eeff_odd | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u0.5_s0.5_r3 |
| diff_stripline | zdiff | 20 | +0.036 | 0.039 | 0.065 | +0.083 | dsl_u0.5_s0.25_r3 |
| diff_stripline | zeven | 20 | +0.037 | 0.037 | 0.055 | +0.072 | dsl_u0.5_s4_r1 |
| embedded | eeff | 24 | +0.000 | 0.001 | 0.002 | +0.005 | emb_u2_d0.05_er4.4 |
| embedded | z0 | 24 | +0.005 | 0.005 | 0.010 | -0.019 | emb_u1_d0.2_er3.66 |
| etch | eeff | 18 | +0.022 | 0.028 | 0.083 | +0.088 | etch_ms_t35_u1_ef3 |
| etch | eeff_even | 3 | +0.010 | 0.010 | 0.017 | +0.018 | etch_dms_ef2 |
| etch | eeff_odd | 3 | +0.016 | 0.016 | 0.022 | +0.023 | etch_dms_ef2 |
| etch | z0 | 18 | -0.014 | 0.045 | 0.082 | +0.104 | etch_ms_t35_u0.5_ef2 |
| etch | zdiff | 3 | +0.014 | 0.014 | 0.024 | +0.026 | etch_dms_ef2 |
| etch | zeven | 3 | +0.006 | 0.007 | 0.008 | +0.008 | etch_dms_ef3 |
| microstrip | eeff | 66 | -0.029 | 0.032 | 0.121 | -0.257 | ms_u0.2_er10.2 |
| microstrip | z0 | 66 | +0.038 | 0.038 | 0.136 | +0.223 | ms_u0.2_er10.2 |
| microstrip_h | eeff | 15 | -0.031 | 0.037 | 0.061 | -0.120 | ms_h1.6_u0.5 |
| microstrip_h | z0 | 15 | +0.053 | 0.053 | 0.116 | +0.238 | ms_h1.6_u0.5 |
| microstrip_mask | eeff | 26 | -0.001 | 0.003 | 0.010 | -0.011 | msm_u1_m10_er4 |
| microstrip_mask | z0 | 26 | +0.025 | 0.025 | 0.035 | +0.036 | msm_u0.5_m10_er4 |
| microstrip_t | eeff | 20 | -0.009 | 0.025 | 0.090 | -0.101 | ms_t17.5_u0.25 |
| microstrip_t | z0 | 20 | +0.033 | 0.033 | 0.104 | +0.123 | ms_t17.5_u0.25 |
| plane_cut | eeff | 16 | -0.090 | 0.090 | 0.169 | -0.180 | cut_w0.2_c4_r3 |
| plane_cut | z0 | 16 | +0.113 | 0.113 | 0.195 | +0.197 | cut_w0.2_c4_r3 |
| plane_edge | eeff | 10 | -0.003 | 0.007 | 0.027 | -0.027 | edge_w0.2_r8 |
| plane_edge | eeff_even | 2 | -0.024 | 0.024 | 0.035 | -0.038 | edge_pair_r2 |
| plane_edge | eeff_odd | 2 | -0.030 | 0.030 | 0.042 | -0.045 | edge_pair_r2 |
| plane_edge | z0 | 10 | +0.024 | 0.024 | 0.035 | +0.048 | edge_w0.2_r8 |
| plane_edge | zdiff | 2 | +0.764 | 0.764 | 1.119 | +1.208 | edge_pair_r2 |
| plane_edge | zeven | 2 | +0.032 | 0.032 | 0.040 | +0.042 | edge_pair_r2 |
| size_study | eeff | 14 | -0.078 | 0.078 | 0.154 | -0.184 | cut_w0.4_c8_r3__m2 |
| size_study | eeff_even | 2 | -0.136 | 0.136 | 0.176 | -0.186 | dms_u1_s4__m1.5 |
| size_study | eeff_odd | 2 | -0.133 | 0.133 | 0.172 | -0.182 | dms_u1_s4__m1.5 |
| size_study | z0 | 14 | +0.103 | 0.103 | 0.195 | +0.222 | ms_h1.6_u1.9__m2 |
| size_study | zdiff | 2 | +0.125 | 0.125 | 0.164 | +0.173 | dms_u1_s4__m1.5 |
| size_study | zeven | 2 | +0.128 | 0.128 | 0.164 | +0.173 | dms_u1_s4__m1.5 |
| stripline | eeff | 29 | +0.000 | 0.000 | 0.000 | +0.000 | sl_h0.1_u2_er4.2 |
| stripline | z0 | 29 | +0.027 | 0.028 | 0.041 | +0.061 | sl_h0.1_u0.25_er4.2 |
| stripline_asym | eeff | 20 | +0.000 | 0.000 | 0.010 | -0.046 | sle_er4.4-2.2_r3 |
| stripline_asym | z0 | 20 | +0.032 | 0.032 | 0.088 | +0.112 | sla_r8_u2 |
| stripline_cpw | eeff | 3 | +0.000 | 0.000 | 0.000 | +0.000 | slcpw_g1 |
| stripline_cpw | z0 | 3 | +0.010 | 0.019 | 0.020 | -0.020 | slcpw_g1 |
| stripline_dual | eeff | 6 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_hm0.1_u0.75 |
| stripline_dual | z0 | 6 | +0.053 | 0.053 | 0.103 | +0.124 | dsl_hm0.4_u0.75 |
| test_board | eeff | 70 | +0.003 | 0.005 | 0.019 | -0.098 | board_tb_jlc4_3313_a_f5d5b2ba96 |
| test_board | eeff_even | 11 | -0.004 | 0.004 | 0.008 | -0.009 | board_tb_jlc4_7628_d_e38643aaf0 |
| test_board | eeff_odd | 11 | -0.002 | 0.002 | 0.005 | -0.010 | board_tb_jlc6_3313_d_4b63fe5ddf |
| test_board | z0 | 70 | +0.034 | 0.034 | 0.131 | +0.306 | board_tb_jlc6_3313_a_79462a6992 |
| test_board | zdiff | 11 | +0.054 | 0.054 | 0.077 | +0.087 | board_tb_jlc6_3313_d_4b63fe5ddf |
| test_board | zeven | 11 | +0.048 | 0.048 | 0.072 | +0.077 | board_tb_jlc6_3313_d_4b63fe5ddf |
| ALL | eeff | 495 | -0.000 | 0.007 | 0.070 | -0.257 | ms_u0.2_er10.2 |
| ALL | eeff_even | 125 | -0.005 | 0.007 | 0.044 | -0.186 | dms_u1_s4__m1.5 |
| ALL | eeff_odd | 125 | -0.014 | 0.015 | 0.063 | -0.182 | dms_u1_s4__m1.5 |
| ALL | z0 | 495 | +0.030 | 0.031 | 0.112 | +0.306 | board_tb_jlc6_3313_a_79462a6992 |
| ALL | zdiff | 125 | +0.048 | 0.048 | 1.071 | +51.297 | board_ts_ea1df30155 |
| ALL | zeven | 125 | +0.032 | 0.039 | 0.144 | -14.526 | board_ts_ea1df30155 |

## FW-M vs CST-M

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.002 | 0.002 | 0.026 | -0.075 | anchor_forsten_suite_5 |
| anchor | eeff_even | 7 | -0.015 | 0.015 | 0.043 | -0.072 | anchor_forsten_suite_7 |
| anchor | eeff_odd | 7 | -0.019 | 0.019 | 0.062 | -0.072 | anchor_forsten_suite_7 |
| anchor | z0 | 16 | +0.072 | 0.072 | 0.249 | +0.397 | anchor_forsten_suite_3 |
| anchor | zdiff | 7 | +0.044 | 0.044 | 0.099 | +0.105 | anchor_thunderscope_xs_4 |
| anchor | zeven | 7 | +0.048 | 0.048 | 0.143 | +0.218 | anchor_forsten_suite_6 |
| cpwg | eeff | 1 | +0.035 | 0.035 | 0.035 | +0.035 | cpwg_u1_g0.5 |
| cpwg | z0 | 1 | -0.028 | 0.028 | 0.028 | -0.028 | cpwg_u1_g0.5 |
| diff_microstrip | eeff_even | 1 | +0.032 | 0.032 | 0.032 | +0.032 | dms_u1_s1 |
| diff_microstrip | eeff_odd | 1 | +0.048 | 0.048 | 0.048 | +0.048 | dms_u1_s1 |
| diff_microstrip | zdiff | 1 | -0.026 | 0.026 | 0.026 | -0.026 | dms_u1_s1 |
| diff_microstrip | zeven | 1 | -0.015 | 0.015 | 0.015 | -0.015 | dms_u1_s1 |
| microstrip | eeff | 1 | +0.042 | 0.042 | 0.042 | +0.042 | ms_u2_er4.4 |
| microstrip | z0 | 1 | -0.026 | 0.026 | 0.026 | -0.026 | ms_u2_er4.4 |
| ALL | eeff | 18 | -0.000 | 0.004 | 0.037 | -0.075 | anchor_forsten_suite_5 |
| ALL | eeff_even | 8 | -0.008 | 0.016 | 0.044 | -0.072 | anchor_forsten_suite_7 |
| ALL | eeff_odd | 8 | -0.016 | 0.027 | 0.060 | -0.072 | anchor_forsten_suite_7 |
| ALL | z0 | 18 | +0.067 | 0.067 | 0.241 | +0.397 | anchor_forsten_suite_3 |
| ALL | zdiff | 8 | +0.042 | 0.042 | 0.098 | +0.105 | anchor_thunderscope_xs_4 |
| ALL | zeven | 8 | +0.043 | 0.043 | 0.130 | +0.218 | anchor_forsten_suite_6 |

## FW open vs CST open

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.002 | 0.002 | 0.026 | -0.075 | anchor_forsten_suite_5 |
| anchor | eeff_even | 7 | -0.015 | 0.015 | 0.045 | -0.072 | anchor_forsten_suite_7 |
| anchor | eeff_odd | 7 | -0.019 | 0.019 | 0.063 | -0.072 | anchor_forsten_suite_7 |
| anchor | z0 | 16 | +0.078 | 0.078 | 0.176 | +0.244 | anchor_forsten_suite_3 |
| anchor | zdiff | 7 | +0.048 | 0.048 | 0.098 | +0.103 | anchor_thunderscope_xs_4 |
| anchor | zeven | 7 | +0.049 | 0.049 | 0.079 | +0.091 | anchor_forsten_suite_7 |
| cpwg | eeff | 1 | +0.035 | 0.035 | 0.035 | +0.035 | cpwg_u1_g0.5 |
| cpwg | z0 | 1 | -0.028 | 0.028 | 0.028 | -0.028 | cpwg_u1_g0.5 |
| diff_microstrip | eeff_even | 1 | +0.032 | 0.032 | 0.032 | +0.032 | dms_u1_s1 |
| diff_microstrip | eeff_odd | 1 | +0.048 | 0.048 | 0.048 | +0.048 | dms_u1_s1 |
| diff_microstrip | zdiff | 1 | -0.026 | 0.026 | 0.026 | -0.026 | dms_u1_s1 |
| diff_microstrip | zeven | 1 | -0.016 | 0.016 | 0.016 | -0.016 | dms_u1_s1 |
| microstrip | eeff | 1 | +0.042 | 0.042 | 0.042 | +0.042 | ms_u2_er4.4 |
| microstrip | z0 | 1 | -0.028 | 0.028 | 0.028 | -0.028 | ms_u2_er4.4 |
| ALL | eeff | 18 | -0.000 | 0.005 | 0.037 | -0.075 | anchor_forsten_suite_5 |
| ALL | eeff_even | 8 | -0.009 | 0.017 | 0.044 | -0.072 | anchor_forsten_suite_7 |
| ALL | eeff_odd | 8 | -0.017 | 0.027 | 0.061 | -0.072 | anchor_forsten_suite_7 |
| ALL | z0 | 18 | +0.070 | 0.070 | 0.174 | +0.244 | anchor_forsten_suite_3 |
| ALL | zdiff | 8 | +0.045 | 0.045 | 0.097 | +0.103 | anchor_thunderscope_xs_4 |
| ALL | zeven | 8 | +0.044 | 0.044 | 0.077 | +0.091 | anchor_forsten_suite_7 |

## FW-E static vs CST-E

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.004 | 0.004 | 0.466 | -0.561 | anchor_forsten_suite_3 |
| anchor | eeff_even | 7 | -0.017 | 0.017 | 0.362 | -0.767 | anchor_forsten_suite_6 |
| anchor | eeff_odd | 7 | -0.019 | 0.019 | 0.102 | -0.140 | anchor_forsten_suite_6 |
| anchor | z0 | 16 | +0.091 | 0.091 | 0.284 | +0.356 | anchor_forsten_suite_3 |
| anchor | zdiff | 7 | +0.083 | 0.083 | 0.098 | +0.101 | anchor_thunderscope_xs_4 |
| anchor | zeven | 7 | +0.052 | 0.052 | 0.176 | +0.294 | anchor_forsten_suite_6 |
| board_other | eeff | 16 | -0.002 | 0.003 | 0.005 | +0.007 | board_ts_9b64c2d2f9 |
| board_other | z0 | 16 | +0.042 | 0.046 | 0.075 | +0.085 | board_vna_a7541acf6b |
| board_target | eeff | 88 | -0.005 | 0.005 | 0.019 | -0.055 | board_vna_79bf5540db |
| board_target | eeff_even | 32 | -0.011 | 0.011 | 0.330 | -0.591 | board_ts_169daecc6a |
| board_target | eeff_odd | 32 | -0.015 | 0.015 | 0.026 | -0.039 | board_ts_169daecc6a |
| board_target | z0 | 88 | +0.025 | 0.026 | 0.083 | +0.139 | board_vna_79bf5540db |
| board_target | zdiff | 32 | +0.047 | 0.047 | 22.485 | +51.305 | board_ts_ea1df30155 |
| board_target | zeven | 32 | +0.000 | 0.031 | 1.414 | -14.373 | board_ts_ea1df30155 |
| broadside | eeff_even | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.1_off0_w0.2 |
| broadside | eeff_odd | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.1_off0_w0.2 |
| broadside | zdiff | 8 | +0.010 | 0.012 | 0.025 | +0.025 | bsl_hm0.1_off0_w0.1 |
| broadside | zeven | 8 | +0.038 | 0.038 | 0.050 | +0.051 | bsl_hm0.1_off0_w0.1 |
| cpw_noplane | eeff | 8 | -0.048 | 0.048 | 0.068 | -0.072 | cpw_w0.3_g0.4 |
| cpw_noplane | z0 | 8 | +0.082 | 0.082 | 0.096 | +0.098 | cpw_w0.3_g0.25 |
| cpwg | eeff | 30 | -0.025 | 0.027 | 0.080 | -0.129 | cpwg_u0.5_g0.25 |
| cpwg | z0 | 30 | +0.047 | 0.047 | 0.116 | +0.155 | cpwg_u1_g0.25 |
| diff_cpwg | eeff_even | 6 | -0.032 | 0.032 | 0.061 | -0.069 | dcpwg_s1_g0.5 |
| diff_cpwg | eeff_odd | 6 | -0.051 | 0.051 | 0.079 | -0.086 | dcpwg_s0.5_g0.5 |
| diff_cpwg | zdiff | 6 | +0.062 | 0.062 | 0.102 | +0.104 | dcpwg_s0.5_g0.5 |
| diff_cpwg | zeven | 6 | +0.052 | 0.052 | 0.084 | +0.089 | dcpwg_s1_g0.5 |
| diff_microstrip | eeff_even | 34 | -0.039 | 0.039 | 0.072 | -0.157 | dms_u0.5_s4 |
| diff_microstrip | eeff_odd | 34 | -0.043 | 0.046 | 0.107 | -0.167 | dms_u0.5_s0.25 |
| diff_microstrip | zdiff | 34 | +0.063 | 0.063 | 0.117 | +0.165 | dms_u0.5_s4 |
| diff_microstrip | zeven | 34 | +0.052 | 0.052 | 0.085 | +0.166 | dms_u0.5_s4 |
| diff_stripline | eeff_even | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u0.5_s0.25_r3 |
| diff_stripline | eeff_odd | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u0.5_s0.5_r3 |
| diff_stripline | zdiff | 20 | +0.036 | 0.039 | 0.065 | +0.083 | dsl_u0.5_s0.25_r3 |
| diff_stripline | zeven | 20 | +0.037 | 0.037 | 0.055 | +0.072 | dsl_u0.5_s4_r1 |
| embedded | eeff | 24 | -0.004 | 0.004 | 0.006 | -0.009 | emb_u2_d0.2_er4.4 |
| embedded | z0 | 24 | +0.007 | 0.008 | 0.012 | -0.017 | emb_u1_d0.2_er3.66 |
| etch | eeff | 18 | +0.015 | 0.023 | 0.078 | +0.082 | etch_ms_t35_u1_ef3 |
| etch | eeff_even | 3 | -0.001 | 0.002 | 0.007 | +0.008 | etch_dms_ef2 |
| etch | eeff_odd | 3 | +0.015 | 0.015 | 0.020 | +0.022 | etch_dms_ef2 |
| etch | z0 | 18 | -0.010 | 0.045 | 0.080 | +0.106 | etch_ms_t35_u0.5_ef2 |
| etch | zdiff | 3 | +0.014 | 0.014 | 0.024 | +0.027 | etch_dms_ef2 |
| etch | zeven | 3 | +0.011 | 0.011 | 0.013 | +0.013 | etch_dms_ef3 |
| microstrip | eeff | 66 | -0.054 | 0.054 | 0.163 | -0.279 | ms_u0.2_er10.2 |
| microstrip | z0 | 66 | +0.069 | 0.069 | 0.148 | +0.234 | ms_u0.2_er10.2 |
| microstrip_h | eeff | 15 | -0.042 | 0.042 | 0.439 | -0.741 | ms_h1.6_u4 |
| microstrip_h | z0 | 15 | +0.062 | 0.062 | 0.333 | +0.356 | ms_h1.6_u0.5 |
| microstrip_mask | eeff | 26 | -0.013 | 0.013 | 0.016 | -0.017 | msm_u1_m10_er4 |
| microstrip_mask | z0 | 26 | +0.030 | 0.030 | 0.038 | +0.039 | msm_u2_m10_er4 |
| microstrip_t | eeff | 20 | -0.016 | 0.030 | 0.094 | -0.104 | ms_t17.5_u0.25 |
| microstrip_t | z0 | 20 | +0.039 | 0.039 | 0.106 | +0.125 | ms_t17.5_u0.25 |
| plane_cut | eeff | 16 | -0.108 | 0.108 | 0.186 | -0.197 | cut_w0.2_c4_r3 |
| plane_cut | z0 | 16 | +0.127 | 0.127 | 0.201 | +0.205 | cut_w0.2_c4_r3 |
| plane_edge | eeff | 10 | -0.022 | 0.022 | 0.031 | -0.038 | edge_w0.2_r8 |
| plane_edge | eeff_even | 2 | -0.052 | 0.052 | 0.061 | -0.063 | edge_pair_r2 |
| plane_edge | eeff_odd | 2 | -0.032 | 0.032 | 0.044 | -0.046 | edge_pair_r2 |
| plane_edge | z0 | 10 | +0.035 | 0.035 | 0.041 | +0.053 | edge_w0.2_r8 |
| plane_edge | zdiff | 2 | +0.765 | 0.765 | 1.120 | +1.209 | edge_pair_r2 |
| plane_edge | zeven | 2 | +0.046 | 0.046 | 0.052 | +0.054 | edge_pair_r2 |
| size_study | eeff | 14 | -0.122 | 0.122 | 0.464 | -0.546 | ms_h1.6_u1.9__m2 |
| size_study | eeff_even | 2 | -0.150 | 0.150 | 0.190 | -0.200 | dms_u1_s4__m1.5 |
| size_study | eeff_odd | 2 | -0.136 | 0.136 | 0.175 | -0.185 | dms_u1_s4__m1.5 |
| size_study | z0 | 14 | +0.121 | 0.121 | 0.347 | +0.447 | ms_h1.6_u1.9__m2 |
| size_study | zdiff | 2 | +0.127 | 0.127 | 0.165 | +0.175 | dms_u1_s4__m1.5 |
| size_study | zeven | 2 | +0.136 | 0.136 | 0.171 | +0.180 | dms_u1_s4__m1.5 |
| stripline | eeff | 29 | +0.000 | 0.000 | 0.000 | +0.000 | sl_h0.1_u2_er4.2 |
| stripline | z0 | 29 | +0.027 | 0.028 | 0.041 | +0.061 | sl_h0.1_u0.25_er4.2 |
| stripline_asym | eeff | 20 | +0.000 | 0.000 | 0.011 | -0.052 | sle_er4.4-2.2_r3 |
| stripline_asym | z0 | 20 | +0.032 | 0.032 | 0.089 | +0.112 | sla_r8_u2 |
| stripline_cpw | eeff | 3 | +0.000 | 0.000 | 0.000 | +0.000 | slcpw_g1 |
| stripline_cpw | z0 | 3 | +0.010 | 0.019 | 0.020 | -0.020 | slcpw_g1 |
| stripline_dual | eeff | 6 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_hm0.1_u0.75 |
| stripline_dual | z0 | 6 | +0.053 | 0.053 | 0.103 | +0.124 | dsl_hm0.4_u0.75 |
| test_board | eeff | 70 | -0.004 | 0.004 | 0.036 | -0.113 | board_tb_jlc4_3313_a_f5d5b2ba96 |
| test_board | eeff_even | 11 | -0.009 | 0.009 | 0.024 | -0.025 | board_tb_jlc4_7628_d_e38643aaf0 |
| test_board | eeff_odd | 11 | -0.004 | 0.004 | 0.008 | -0.010 | board_tb_jlc6_3313_d_4b63fe5ddf |
| test_board | z0 | 70 | +0.038 | 0.038 | 0.141 | +0.314 | board_tb_jlc6_3313_a_79462a6992 |
| test_board | zdiff | 11 | +0.055 | 0.055 | 0.077 | +0.087 | board_tb_jlc6_3313_d_4b63fe5ddf |
| test_board | zeven | 11 | +0.057 | 0.057 | 0.073 | +0.078 | board_tb_jlc6_3313_d_4b63fe5ddf |
| ALL | eeff | 495 | -0.006 | 0.007 | 0.101 | -0.741 | ms_h1.6_u4 |
| ALL | eeff_even | 125 | -0.013 | 0.013 | 0.072 | -0.767 | anchor_forsten_suite_6 |
| ALL | eeff_odd | 125 | -0.015 | 0.016 | 0.070 | -0.185 | dms_u1_s4__m1.5 |
| ALL | z0 | 495 | +0.036 | 0.036 | 0.125 | +0.447 | ms_h1.6_u1.9__m2 |
| ALL | zdiff | 125 | +0.049 | 0.049 | 1.071 | +51.305 | board_ts_ea1df30155 |
| ALL | zeven | 125 | +0.037 | 0.042 | 0.139 | -14.373 | board_ts_ea1df30155 |

## FW-E vs FDM-E

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.284 | 0.392 | 0.473 | -0.547 | anchor_stripline_asym_6 |
| anchor | eeff_even | 7 | -0.176 | 0.258 | 0.544 | +0.636 | anchor_forsten_suite_6 |
| anchor | eeff_odd | 7 | -0.318 | 0.318 | 0.924 | -1.028 | anchor_thunderscope_xs_3 |
| anchor | z0 | 16 | +0.057 | 0.061 | 0.217 | +0.259 | anchor_thunderscope_xs_6 |
| anchor | zdiff | 7 | +0.421 | 0.421 | 0.789 | +0.863 | anchor_forsten_suite_6 |
| anchor | zeven | 7 | +0.208 | 0.208 | 0.443 | +0.556 | anchor_thunderscope_xs_5 |
| board_other | eeff | 16 | -0.389 | 0.389 | 0.554 | -0.772 | board_ts_9b64c2d2f9 |
| board_other | z0 | 16 | +0.070 | 0.070 | 0.136 | +0.158 | board_ts_64b2db8632 |
| board_target | eeff | 88 | -0.355 | 0.355 | 0.505 | -0.815 | board_ts_a223d492c7 |
| board_target | eeff_even | 32 | -0.342 | 0.347 | 0.460 | -0.499 | board_ts_01906bbc96 |
| board_target | eeff_odd | 32 | -0.587 | 0.587 | 1.012 | -1.084 | board_ts_01906bbc96 |
| board_target | z0 | 88 | +0.028 | 0.028 | 0.188 | +0.351 | board_ts_017660f875 |
| board_target | zdiff | 32 | +0.211 | 0.211 | 0.518 | +0.550 | board_ts_9280b49146 |
| board_target | zeven | 32 | +0.117 | 0.117 | 0.412 | +0.534 | board_ts_ea1df30155 |
| broadside | eeff_even | 8 | -0.285 | 0.285 | 0.334 | -0.339 | bsl_hm0.2_off0.15_w0.1 |
| broadside | eeff_odd | 8 | -0.453 | 0.453 | 0.529 | -0.549 | bsl_hm0.1_off0_w0.1 |
| broadside | zdiff | 8 | +0.363 | 0.363 | 0.486 | +0.490 | bsl_hm0.2_off0_w0.1 |
| broadside | zeven | 8 | +0.282 | 0.282 | 0.372 | +0.396 | bsl_hm0.2_off0.15_w0.1 |
| cpw_noplane | eeff | 8 | +0.044 | 0.044 | 0.073 | +0.075 | cpw_w0.3_g0.15 |
| cpw_noplane | z0 | 8 | +0.273 | 0.273 | 0.344 | +0.397 | cpw_w0.6_g0.1 |
| cpwg | eeff | 30 | -0.090 | 0.362 | 0.694 | -0.941 | cpwg_u0.5_g0.25_mask |
| cpwg | z0 | 30 | +0.071 | 0.071 | 0.173 | +0.226 | cpwg_u2_g0.25 |
| diff_cpwg | eeff_even | 6 | +0.256 | 0.256 | 0.290 | +0.309 | dcpwg_s1_g0.5 |
| diff_cpwg | eeff_odd | 6 | +0.169 | 0.169 | 0.204 | +0.232 | dcpwg_s1_g0.5 |
| diff_cpwg | zdiff | 6 | +0.342 | 0.342 | 0.447 | +0.466 | dcpwg_s0.5_g0.5 |
| diff_cpwg | zeven | 6 | +0.212 | 0.212 | 0.333 | +0.342 | dcpwg_s1_g0.5 |
| diff_microstrip | eeff_even | 34 | +0.089 | 0.217 | 0.416 | -0.505 | dms_u0.5_s4_mask |
| diff_microstrip | eeff_odd | 34 | -0.019 | 0.250 | 0.938 | -1.245 | dms_u0.5_s0.25_mask |
| diff_microstrip | zdiff | 34 | +0.515 | 0.515 | 0.707 | +0.968 | dms_u2_s0.25 |
| diff_microstrip | zeven | 34 | +0.301 | 0.301 | 0.464 | +0.673 | dms_u0.5_s4 |
| diff_stripline | eeff_even | 20 | -0.325 | 0.325 | 0.458 | -0.483 | dsl_u0.5_s2_r1 |
| diff_stripline | eeff_odd | 20 | -0.527 | 0.527 | 0.850 | -0.864 | dsl_u0.5_s0.25_r3 |
| diff_stripline | zdiff | 20 | +0.466 | 0.466 | 0.705 | +0.868 | dsl_u0.5_s0.25_r3 |
| diff_stripline | zeven | 20 | +0.301 | 0.301 | 0.432 | +0.505 | dsl_u0.5_s4_r3 |
| embedded | eeff | 24 | -0.323 | 0.323 | 0.425 | -0.473 | emb_u0.5_d0.05_er3.66 |
| embedded | z0 | 24 | +0.058 | 0.058 | 0.091 | +0.100 | emb_u0.5_d0.5_er3.66 |
| microstrip | eeff | 66 | +0.170 | 0.170 | 0.800 | +2.010 | ms_u0.1_er10.2 |
| microstrip | z0 | 66 | +0.158 | 0.158 | 0.243 | +0.327 | ms_u0.1_er10.2 |
| microstrip_h | eeff | 15 | +0.300 | 0.300 | 0.857 | +1.462 | ms_h0.07_u0.5 |
| microstrip_h | z0 | 15 | +0.100 | 0.100 | 0.194 | +0.244 | ms_h1.6_u0.5 |
| microstrip_mask | eeff | 26 | -0.282 | 0.282 | 0.555 | -0.623 | msm_u0.5_m10_er4 |
| microstrip_mask | z0 | 26 | +0.044 | 0.044 | 0.069 | +0.072 | msm_u1_m10_er4 |
| microstrip_t | eeff | 20 | +0.275 | 0.275 | 0.803 | +0.878 | ms_t35_u0.25 |
| microstrip_t | z0 | 20 | +0.117 | 0.117 | 0.201 | +0.206 | ms_t17.5_u0.25 |
| plane_cut | eeff | 16 | +0.153 | 0.153 | 0.245 | +0.251 | cut_w0.2_c1_r1 |
| plane_cut | z0 | 16 | +0.136 | 0.136 | 0.207 | +0.237 | cut_w0.4_c1_r3 |
| plane_edge | eeff | 10 | +0.240 | 0.240 | 0.285 | +0.286 | edge_w0.2_r1 |
| plane_edge | eeff_even | 2 | +0.299 | 0.299 | 0.325 | +0.332 | edge_pair_r2 |
| plane_edge | eeff_odd | 2 | +0.113 | 0.113 | 0.147 | +0.155 | edge_pair_r2 |
| plane_edge | z0 | 10 | +0.079 | 0.079 | 0.111 | +0.112 | edge_w0.2_r8 |
| plane_edge | zdiff | 2 | +0.473 | 0.473 | 0.477 | +0.478 | edge_pair_r0.5 |
| plane_edge | zeven | 2 | +0.298 | 0.298 | 0.304 | +0.305 | edge_pair_r0.5 |
| size_study | eeff | 14 | +0.175 | 0.262 | 0.453 | -0.562 | sla_r8_u1__m2 |
| size_study | eeff_even | 2 | +0.166 | 0.166 | 0.188 | +0.194 | dms_u1_s4__m1.5 |
| size_study | eeff_odd | 2 | +0.120 | 0.120 | 0.141 | +0.146 | dms_u1_s4__m1.5 |
| size_study | z0 | 14 | +0.112 | 0.112 | 0.386 | +0.656 | sla_r8_u1__m2 |
| size_study | zdiff | 2 | +0.655 | 0.655 | 0.693 | +0.703 | dms_u1_s4__m2 |
| size_study | zeven | 2 | +0.612 | 0.612 | 0.648 | +0.657 | dms_u1_s4__m2 |
| stripline | eeff | 29 | -0.358 | 0.358 | 0.501 | -0.608 | sl_h0.1_u0.5_er3 |
| stripline | z0 | 29 | +0.048 | 0.048 | 0.102 | +0.106 | sl_h0.2_u0.25_er3 |
| stripline_asym | eeff | 20 | -0.445 | 0.445 | 0.608 | -0.644 | sla_r8_u0.5 |
| stripline_asym | z0 | 20 | +0.068 | 0.068 | 0.164 | +0.306 | sla_r8_u0.5 |
| stripline_cpw | eeff | 3 | -0.592 | 0.592 | 0.718 | -0.750 | slcpw_g0.5 |
| stripline_cpw | z0 | 3 | -0.007 | 0.041 | 0.066 | +0.073 | slcpw_g2 |
| stripline_dual | eeff | 6 | -0.337 | 0.337 | 0.381 | -0.386 | dsl_hm0.2_u0.75 |
| stripline_dual | z0 | 6 | +0.075 | 0.075 | 0.114 | +0.143 | dsl_hm0.4_u0.75 |
| test_board | eeff | 70 | -0.333 | 0.333 | 0.541 | -0.771 | board_tb_jlc4_3313_a_f5d5b2ba96 |
| test_board | eeff_even | 11 | -0.276 | 0.276 | 0.369 | -0.485 | board_tb_jlc4_3313_d_1940d275c1 |
| test_board | eeff_odd | 11 | -0.358 | 0.358 | 0.646 | -0.790 | board_tb_jlc4_3313_d_1940d275c1 |
| test_board | z0 | 70 | +0.062 | 0.062 | 0.139 | +0.200 | board_tb_aisler4_assumed_a_1ed517f68f |
| test_board | zdiff | 11 | +0.334 | 0.334 | 0.696 | +0.727 | board_tb_jlc6_3313_d_d082c201fe |
| test_board | zeven | 11 | +0.311 | 0.311 | 0.525 | +0.537 | board_tb_jlc6_7628_d_1be5f0ab4c |
| ALL | eeff | 477 | -0.262 | 0.305 | 0.567 | +2.010 | ms_u0.1_er10.2 |
| ALL | eeff_even | 122 | -0.263 | 0.299 | 0.452 | +0.636 | anchor_forsten_suite_6 |
| ALL | eeff_odd | 122 | -0.427 | 0.427 | 0.869 | -1.245 | dms_u0.5_s0.25_mask |
| ALL | z0 | 477 | +0.068 | 0.069 | 0.211 | +0.656 | sla_r8_u1__m2 |
| ALL | zdiff | 122 | +0.421 | 0.421 | 0.696 | +0.968 | dms_u2_s0.25 |
| ALL | zeven | 122 | +0.284 | 0.284 | 0.457 | +0.673 | dms_u0.5_s4 |

## FW-M vs FDM-M

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | -0.283 | 0.283 | 0.436 | -0.545 | anchor_stripline_asym_6 |
| anchor | eeff_even | 7 | -0.180 | 0.251 | 0.364 | -0.437 | anchor_thunderscope_xs_3 |
| anchor | eeff_odd | 7 | -0.324 | 0.324 | 0.932 | -1.049 | anchor_thunderscope_xs_3 |
| anchor | z0 | 16 | +0.085 | 0.085 | 0.249 | +0.254 | anchor_thunderscope_xs_6 |
| anchor | zdiff | 7 | +0.413 | 0.413 | 0.840 | +1.067 | anchor_forsten_suite_6 |
| anchor | zeven | 7 | +0.269 | 0.269 | 0.520 | +0.529 | anchor_forsten_suite_7 |
| board_other | eeff | 16 | -0.379 | 0.379 | 0.581 | -0.807 | board_ts_9b64c2d2f9 |
| board_other | z0 | 16 | +0.086 | 0.086 | 0.143 | +0.167 | board_ts_64b2db8632 |
| board_target | eeff | 88 | -0.346 | 0.346 | 0.499 | -0.822 | board_ts_a223d492c7 |
| board_target | eeff_even | 30 | -0.334 | 0.339 | 0.447 | +1.249 | board_ts_169daecc6a |
| board_target | eeff_odd | 30 | -0.597 | 0.597 | 1.040 | -1.104 | board_ts_01906bbc96 |
| board_target | z0 | 88 | +0.076 | 0.076 | 0.200 | +0.402 | board_ts_017660f875 |
| board_target | zdiff | 30 | +0.340 | 0.340 | 0.554 | +0.599 | board_ts_471a028ef5 |
| board_target | zeven | 30 | +0.224 | 0.230 | 0.414 | +0.466 | board_ts_471a028ef5 |
| broadside | eeff_even | 8 | -0.277 | 0.277 | 0.353 | -0.358 | bsl_hm0.2_off0.15_w0.1 |
| broadside | eeff_odd | 8 | -0.446 | 0.446 | 0.552 | -0.578 | bsl_hm0.1_off0_w0.1 |
| broadside | zdiff | 8 | +0.359 | 0.359 | 0.446 | +0.477 | bsl_hm0.2_off0.15_w0.1 |
| broadside | zeven | 8 | +0.269 | 0.269 | 0.342 | +0.391 | bsl_hm0.2_off0.15_w0.1 |
| cpw_noplane | eeff | 8 | -0.011 | 0.025 | 0.056 | -0.087 | cpw_w0.6_g0.1 |
| cpw_noplane | z0 | 8 | +0.594 | 0.594 | 0.770 | +0.902 | cpw_w0.6_g0.1 |
| cpwg | eeff | 30 | -0.108 | 0.367 | 0.744 | -1.070 | cpwg_u0.5_g0.25_mask |
| cpwg | z0 | 30 | +0.107 | 0.107 | 0.199 | +0.301 | cpwg_u1_g0.25 |
| diff_cpwg | eeff_even | 6 | +0.225 | 0.225 | 0.258 | +0.261 | dcpwg_s1_g2 |
| diff_cpwg | eeff_odd | 6 | +0.100 | 0.100 | 0.145 | +0.160 | dcpwg_s1_g0.5 |
| diff_cpwg | zdiff | 6 | +0.449 | 0.449 | 0.560 | +0.561 | dcpwg_s1_g0.5 |
| diff_cpwg | zeven | 6 | +0.295 | 0.295 | 0.398 | +0.445 | dcpwg_s1_g0.5 |
| diff_microstrip | eeff_even | 34 | +0.106 | 0.262 | 0.448 | +0.554 | dms_u0.5_s1 |
| diff_microstrip | eeff_odd | 34 | -0.011 | 0.255 | 0.926 | -1.271 | dms_u0.5_s0.25_mask |
| diff_microstrip | zdiff | 34 | +0.671 | 0.671 | 0.971 | +1.151 | dms_u1_s0.25 |
| diff_microstrip | zeven | 34 | +0.412 | 0.412 | 0.603 | +0.762 | dms_u0.5_s4 |
| diff_stripline | eeff_even | 20 | -0.313 | 0.313 | 0.454 | -0.494 | dsl_u0.5_s2_r1 |
| diff_stripline | eeff_odd | 20 | -0.518 | 0.518 | 0.857 | -1.025 | dsl_u0.5_s0.25_r1 |
| diff_stripline | zdiff | 20 | +0.447 | 0.447 | 0.683 | +0.874 | dsl_u0.5_s0.25_r3 |
| diff_stripline | zeven | 20 | +0.294 | 0.294 | 0.426 | +0.498 | dsl_u0.5_s4_r3 |
| embedded | eeff | 24 | -0.253 | 0.253 | 0.365 | -0.422 | emb_u0.5_d0.05_er4.4 |
| embedded | z0 | 24 | +0.115 | 0.115 | 0.137 | +0.142 | emb_u0.5_d0.1_er3.66 |
| microstrip | eeff | 66 | +0.187 | 0.187 | 0.806 | +2.084 | ms_u0.1_er10.2 |
| microstrip | z0 | 66 | +0.175 | 0.175 | 0.285 | +0.350 | ms_u0.1_er3 |
| microstrip_h | eeff | 15 | +0.255 | 0.255 | 0.872 | +1.528 | ms_h0.07_u0.5 |
| microstrip_h | z0 | 15 | +0.131 | 0.131 | 0.229 | +0.274 | ms_h1.6_u0.5 |
| microstrip_mask | eeff | 26 | -0.264 | 0.264 | 0.529 | -0.599 | msm_u0.5_m10_er4 |
| microstrip_mask | z0 | 26 | +0.135 | 0.135 | 0.173 | +0.179 | msm_u1_m20_er4 |
| microstrip_t | eeff | 20 | +0.324 | 0.324 | 0.813 | +0.952 | ms_t35_u0.25 |
| microstrip_t | z0 | 20 | +0.149 | 0.149 | 0.259 | +0.338 | ms_t17.5_u0.25 |
| plane_cut | eeff | 16 | +0.123 | 0.123 | 0.232 | +0.237 | cut_w0.2_c1_r1 |
| plane_cut | z0 | 16 | +0.201 | 0.201 | 0.261 | +0.276 | cut_w0.4_c1_r3 |
| plane_edge | eeff | 10 | +0.244 | 0.244 | 0.290 | +0.298 | edge_w0.2_r1 |
| plane_edge | eeff_even | 2 | +0.329 | 0.329 | 0.346 | +0.350 | edge_pair_r2 |
| plane_edge | eeff_odd | 2 | +0.046 | 0.050 | 0.087 | +0.096 | edge_pair_r2 |
| plane_edge | z0 | 10 | +0.259 | 0.259 | 0.293 | +0.338 | edge_w0.2_r0.5 |
| plane_edge | zdiff | 2 | +0.771 | 0.771 | 0.777 | +0.779 | edge_pair_r0.5 |
| plane_edge | zeven | 2 | +0.593 | 0.593 | 0.631 | +0.641 | edge_pair_r0.5 |
| size_study | eeff | 14 | +0.134 | 0.223 | 0.482 | -0.572 | sla_r8_u1__m2 |
| size_study | eeff_even | 2 | +0.154 | 0.154 | 0.166 | +0.169 | dms_u1_s4__m1.5 |
| size_study | eeff_odd | 2 | +0.108 | 0.108 | 0.115 | +0.116 | dms_u1_s4__m1.5 |
| size_study | z0 | 14 | +0.148 | 0.148 | 0.357 | +0.643 | sla_r8_u1__m2 |
| size_study | zdiff | 2 | +0.808 | 0.808 | 0.828 | +0.834 | dms_u1_s4__m2 |
| size_study | zeven | 2 | +0.766 | 0.766 | 0.779 | +0.783 | dms_u1_s4__m2 |
| stripline | eeff | 29 | -0.343 | 0.343 | 0.520 | -0.635 | sl_h0.1_u0.5_er3 |
| stripline | z0 | 29 | +0.044 | 0.044 | 0.090 | +0.106 | sl_h0.2_u0.25_er3 |
| stripline_asym | eeff | 20 | -0.428 | 0.428 | 0.631 | -0.673 | sla_r8_u0.5 |
| stripline_asym | z0 | 20 | +0.067 | 0.067 | 0.161 | +0.295 | sla_r8_u0.5 |
| stripline_cpw | eeff | 3 | -0.603 | 0.603 | 0.725 | -0.755 | slcpw_g0.5 |
| stripline_cpw | z0 | 3 | -0.008 | 0.029 | 0.063 | +0.072 | slcpw_g2 |
| stripline_dual | eeff | 6 | -0.353 | 0.353 | 0.417 | -0.453 | dsl_hm0.2_u0.75 |
| stripline_dual | z0 | 6 | +0.057 | 0.057 | 0.109 | +0.134 | dsl_hm0.4_u0.75 |
| test_board | eeff | 70 | -0.323 | 0.323 | 0.494 | -0.712 | board_tb_jlc4_3313_a_f5d5b2ba96 |
| test_board | eeff_even | 11 | -0.211 | 0.211 | 0.383 | -0.446 | board_tb_jlc4_3313_d_1940d275c1 |
| test_board | eeff_odd | 11 | -0.389 | 0.389 | 0.646 | -0.798 | board_tb_jlc4_3313_d_1940d275c1 |
| test_board | z0 | 70 | +0.093 | 0.093 | 0.177 | +0.258 | board_tb_jlc4_7628_f3_7e689f5994 |
| test_board | zdiff | 11 | +0.534 | 0.534 | 0.711 | +0.730 | board_tb_jlc6_3313_d_d082c201fe |
| test_board | zeven | 11 | +0.497 | 0.497 | 0.548 | +0.586 | board_tb_jlc4_7628_d_e38643aaf0 |
| ALL | eeff | 477 | -0.239 | 0.298 | 0.563 | +2.084 | ms_u0.1_er10.2 |
| ALL | eeff_even | 120 | -0.264 | 0.300 | 0.438 | +1.249 | board_ts_169daecc6a |
| ALL | eeff_odd | 120 | -0.434 | 0.434 | 0.918 | -1.271 | dms_u0.5_s0.25_mask |
| ALL | z0 | 477 | +0.115 | 0.115 | 0.254 | +0.902 | cpw_w0.6_g0.1 |
| ALL | zdiff | 120 | +0.505 | 0.505 | 0.834 | +1.151 | dms_u1_s0.25 |
| ALL | zeven | 120 | +0.329 | 0.334 | 0.554 | +0.783 | dms_u1_s4__m2 |

## dispersion 1 GHz

| group | quantity | n | bias % | med % | p90 % | max % | max case |
|---|---|---:|---:|---:|---:|---:|---|
| anchor | eeff | 16 | +0.001 | 0.001 | 0.456 | +0.532 | anchor_forsten_suite_3 |
| anchor | eeff_even | 7 | +0.003 | 0.003 | 0.313 | +0.743 | anchor_forsten_suite_6 |
| anchor | eeff_odd | 7 | +0.000 | 0.000 | 0.044 | +0.105 | anchor_forsten_suite_6 |
| board_other | eeff | 16 | +0.000 | 0.000 | 0.004 | +0.009 | board_vna_fe17140140 |
| board_target | eeff | 88 | +0.009 | 0.009 | 0.016 | +0.039 | board_ts_3e274a1681 |
| board_target | eeff_even | 32 | +0.004 | 0.004 | 0.330 | +0.580 | board_ts_169daecc6a |
| board_target | eeff_odd | 32 | +0.000 | 0.000 | 0.006 | +0.016 | board_ts_834d341d7a |
| broadside | eeff_even | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.2_off0.15_w0.1 |
| broadside | eeff_odd | 8 | +0.000 | 0.000 | 0.000 | +0.000 | bsl_hm0.2_off0.15_w0.1 |
| cpw_noplane | eeff | 8 | +0.005 | 0.005 | 0.010 | +0.014 | cpw_w0.6_g0.4 |
| cpwg | eeff | 30 | +0.007 | 0.007 | 0.011 | +0.012 | cpwg_u2_g1 |
| diff_cpwg | eeff_even | 6 | +0.013 | 0.013 | 0.014 | +0.014 | dcpwg_s1_g1 |
| diff_cpwg | eeff_odd | 6 | +0.001 | 0.001 | 0.002 | +0.002 | dcpwg_s1_g0.5 |
| diff_microstrip | eeff_even | 34 | +0.010 | 0.010 | 0.018 | +0.020 | dms_u2_s4_mask |
| diff_microstrip | eeff_odd | 34 | +0.001 | 0.001 | 0.003 | +0.004 | dms_u2_s4 |
| diff_stripline | eeff_even | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u1_s4_r3 |
| diff_stripline | eeff_odd | 20 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_u1_s4_r3 |
| embedded | eeff | 24 | +0.005 | 0.005 | 0.007 | +0.009 | emb_u2_d0.05_er4.4 |
| etch | eeff | 18 | +0.006 | 0.006 | 0.009 | +0.009 | etch_ms_t70_u2_ef3 |
| etch | eeff_even | 3 | +0.010 | 0.010 | 0.010 | +0.010 | etch_dms_ef3 |
| etch | eeff_odd | 3 | +0.001 | 0.001 | 0.001 | +0.001 | etch_dms_ef3 |
| microstrip | eeff | 66 | +0.013 | 0.013 | 0.057 | +0.208 | ms_u10_er10.2 |
| microstrip_h | eeff | 15 | +0.009 | 0.009 | 0.361 | +0.704 | ms_h1.6_u4 |
| microstrip_mask | eeff | 26 | +0.009 | 0.009 | 0.015 | +0.015 | msm_u4_m35_er4 |
| microstrip_t | eeff | 20 | +0.006 | 0.006 | 0.015 | +0.017 | ms_t105_u4 |
| plane_cut | eeff | 16 | +0.017 | 0.017 | 0.045 | +0.082 | cut_w0.4_c8_r3 |
| plane_edge | eeff | 10 | +0.018 | 0.018 | 0.025 | +0.027 | edge_w0.4_r0.5 |
| plane_edge | eeff_even | 2 | +0.028 | 0.028 | 0.030 | +0.031 | edge_pair_r0.5 |
| plane_edge | eeff_odd | 2 | +0.002 | 0.002 | 0.002 | +0.002 | edge_pair_r0.5 |
| size_study | eeff | 14 | +0.018 | 0.018 | 0.352 | +0.455 | ms_h1.6_u1.9__m1.5 |
| size_study | eeff_even | 2 | +0.015 | 0.015 | 0.015 | +0.015 | dms_u1_s4__m2 |
| size_study | eeff_odd | 2 | +0.003 | 0.003 | 0.003 | +0.003 | dms_u1_s4__m2 |
| stripline | eeff | 29 | +0.000 | 0.000 | 0.000 | +0.000 | sl_h0.2_u0.5_er4.2 |
| stripline_asym | eeff | 20 | +0.000 | 0.000 | 0.003 | +0.006 | sle_er4.4-2.2_r3 |
| stripline_cpw | eeff | 3 | +0.000 | 0.000 | 0.000 | +0.000 | slcpw_g0.5 |
| stripline_dual | eeff | 6 | +0.000 | 0.000 | 0.000 | +0.000 | dsl_hm0.2_u1.5 |
| test_board | eeff | 70 | +0.009 | 0.009 | 0.016 | +0.019 | board_tb_aisler4_assumed_a_224e65ed41 |
| test_board | eeff_even | 11 | +0.003 | 0.003 | 0.019 | +0.019 | board_tb_jlc8_7628_c_8d1d2929d8 |
| test_board | eeff_odd | 11 | +0.000 | 0.000 | 0.003 | +0.003 | board_tb_jlc6_7628_c_363ea84ec9 |
| ALL | eeff | 495 | +0.007 | 0.007 | 0.022 | +0.704 | ms_h1.6_u4 |
| ALL | eeff_even | 125 | +0.006 | 0.006 | 0.019 | +0.743 | anchor_forsten_suite_6 |
| ALL | eeff_odd | 125 | +0.001 | 0.001 | 0.003 | +0.105 | anchor_forsten_suite_6 |

## lossy sweep vs CST lossy (frequency domain, E walls)

| quantity | f GHz | n | bias % | med % | p90 % | max % | max case |
|---|---:|---:|---:|---:|---:|---:|---|
| alpha | 0.1 | 31 | +15.84 | 15.84 | 22.75 | +25.29 | anchor_forsten_suite_10:odd |
| alpha | 1 | 31 | +9.77 | 9.77 | 16.12 | +24.99 | anchor_forsten_suite_6:odd |
| alpha | 5 | 30 | +7.12 | 7.12 | 14.38 | +17.21 | anchor_forsten_suite_10:odd |
| alpha | 10 | 30 | +4.32 | 4.57 | 15.82 | +19.24 | anchor_forsten_suite_6:odd |
| alpha | 20 | 28 | +5.40 | 6.36 | 16.03 | +33.04 | anchor_forsten_suite_6:odd |
| alpha | 40 | 25 | +4.62 | 6.82 | 17.35 | +52.19 | anchor_forsten_suite_6:odd |
| eeff | 0.1 | 31 | -0.55 | 0.55 | 1.09 | -2.63 | ms_h1.6_u1.9:single |
| eeff | 1 | 31 | -0.48 | 0.48 | 1.06 | -1.97 | anchor_forsten_suite_6:odd |
| eeff | 5 | 30 | -0.68 | 0.68 | 1.02 | -2.62 | ms_h1.6_u1.9:single |
| eeff | 10 | 30 | -0.62 | 0.62 | 0.94 | -2.05 | ms_h1.6_u1.9:single |
| eeff | 20 | 28 | -0.71 | 0.71 | 1.44 | -1.53 | ms_h0.2_u1.9:single |
| eeff | 40 | 25 | -0.78 | 0.78 | 1.16 | -1.91 | ms_h0.2_u1.9:single |
| zc_re | 0.1 | 31 | +0.22 | 0.22 | 0.40 | +0.58 | cpwg_u1_g0.5:single |
| zc_re | 1 | 31 | +0.07 | 0.07 | 0.13 | +0.17 | dms_u1_s1:odd |
| zc_re | 5 | 30 | +0.04 | 0.07 | 0.72 | -2.43 | anchor_forsten_suite_6:even |
| zc_re | 10 | 30 | -0.08 | 0.09 | 2.43 | -6.46 | anchor_forsten_suite_6:even |
| zc_re | 20 | 28 | -0.17 | 0.31 | 13.72 | -43.31 | anchor_forsten_suite_8:even |
| zc_re | 40 | 25 | -0.69 | 0.80 | 14.68 | -45.61 | anchor_forsten_suite_6:odd |

Left out: 11 CST points whose line impedance is off its lowest-frequency value by more than 2x (CST lost the quasi-TEM mode): anchor_forsten_suite_3:single@5, anchor_forsten_suite_3:single@10, anchor_forsten_suite_3:single@20, anchor_forsten_suite_3:single@40, anchor_forsten_suite_5:single@20, anchor_forsten_suite_5:single@40, anchor_forsten_suite_8:odd@20, anchor_forsten_suite_8:odd@40, anchor_forsten_suite_8:even@40, cpwg_u1_g0.5:single@40, board_vna_689da5c270:single@40

## run time per task (s, one node process on one vCPU)

| kind | n | median | p90 | max | max case |
|---|---:|---:|---:|---:|---|
| E | 620 | 6 | 13 | 126 | edge_pair_r2 |
| M | 618 | 6 | 12 | 60 | edge_pair_r2 |
| sweep | 620 | 13 | 53 | 356 | board_ts_834d341d7a |

## sweep checks (620 sweeps)

Warnings of the backend (type: number of cases):

* accuracy: 8 (anchor_forsten_suite_3, dsl_u1_s0.25_r1, dsl_u1_s0.25_r3, dsl_u1_s0.5_r1, dsl_u1_s0.5_r3, board_ts_ea1df30155, ms_h1.6_u4, board_ts_860e846530)
* eigen-ambiguous: 112
* wall-tied-grounds: 243

Sweeps whose eigenmode εeff jumps by more than 2.0 % between neighbouring points (first jump): 178

* cut_w0.4_c8_r3__m2:single at 3.162 GHz: -5.0 %
* board_ts_169daecc6a:even at 3.162 GHz: +2.0 %
* cut_w0.4_c8_r3__m1.5:single at 3.981 GHz: +3.4 %
* board_ts_54b3d93297:even at 3.981 GHz: -12.1 %
* board_tb_jlc6_3313_a_9de967b93a:single at 5 GHz: -2.7 %
* board_tb_jlc6_3313_a_47708d7717:single at 5 GHz: -2.9 %
* board_tb_jlc6_3313_a_79462a6992:single at 5 GHz: -3.1 %
* board_ts_3e274a1681:single at 5 GHz: -3.8 %
* board_ts_e3c2afe76d:single at 5 GHz: -3.8 %
* board_ts_834d341d7a:odd at 5 GHz: -2.2 %
* board_ts_834d341d7a:even at 5 GHz: -3.8 %
* cut_w0.4_c4_r3:single at 6.31 GHz: -5.2 %
* cut_w0.2_c8_r3:single at 6.31 GHz: -3.9 %
* cut_w0.4_c8_r3:single at 6.31 GHz: -7.5 %
* board_ts_017660f875:single at 6.31 GHz: -2.2 %
* board_vna_8401fa4455:single at 7.943 GHz: +4.2 %
* board_vna_79bf5540db:single at 7.943 GHz: +4.2 %
* anchor_forsten_suite_1:single at 10 GHz: +2.0 %
* anchor_forsten_suite_3:single at 10 GHz: +2.6 %
* anchor_forsten_suite_6:odd at 10 GHz: +2.4 %
* anchor_forsten_suite_5:single at 12.59 GHz: +3.0 %
* cut_w0.4_c2_r1:single at 12.59 GHz: -3.7 %
* cut_w0.2_c4_r1:single at 12.59 GHz: -2.5 %
* cut_w0.4_c4_r1:single at 12.59 GHz: -6.5 %
* board_ts_ea1df30155:even at 12.59 GHz: +2.2 %
* cut_w0.2_c8_r1:single at 12.59 GHz: -4.3 %
* cut_w0.4_c8_r1:single at 12.59 GHz: +3.0 %
* board_ts_860e846530:even at 12.59 GHz: +2.2 %
* anchor_forsten_suite_8:odd at 15.85 GHz: +2.4 %
* anchor_forsten_suite_8:even at 15.85 GHz: +3.6 %
* ms_h1.6_u1.9:single at 15.85 GHz: +2.1 %
* ms_h1.6_u1.9__m1.5:single at 15.85 GHz: +2.1 %
* ms_h1.6_u1.9__m2:single at 15.85 GHz: +2.1 %
* cpwg_u1_g4__m2:single at 15.85 GHz: -3.5 %
* board_ts_48744aa336:single at 15.85 GHz: -2.3 %
* board_vna_731023bc00:single at 15.85 GHz: +2.5 %
* ms_h1.6_u0.5:single at 15.85 GHz: +2.1 %
* board_ts_98245e1d36:single at 15.85 GHz: -2.8 %
* cpwg_u1_g0.5:single at 20 GHz: +3.6 %
* cpwg_u0.5_g0.25:single at 20 GHz: +3.7 %
* cpwg_u0.5_g0.5:single at 20 GHz: +3.3 %
* cpwg_u0.5_g1:single at 20 GHz: +2.8 %
* cpwg_u1_g0.25:single at 20 GHz: +4.6 %
* cpwg_u1_g1:single at 20 GHz: +2.7 %
* cpwg_u2_g0.25:single at 20 GHz: +5.0 %
* cpwg_u2_g0.5:single at 20 GHz: +3.6 %
* cpwg_u2_g0.5_mask:single at 20 GHz: +2.0 %
* cpwg_u2_g1:single at 20 GHz: +2.5 %
* dcpwg_s0.5_g1:even at 20 GHz: +2.4 %
* dcpwg_s1_g1:even at 20 GHz: +2.3 %
* cpwg_u1_g4__m1.5:single at 20 GHz: -4.4 %
* board_vna_689da5c270:single at 20 GHz: +2.4 %
* board_vna_b95d8b0e2e:single at 20 GHz: +2.5 %
* board_vna_77c34e31bc:single at 20 GHz: +2.5 %
* board_vna_e96f6ed159:single at 20 GHz: +2.3 %
* board_vna_475db2799d:single at 20 GHz: +2.3 %
* board_vna_82efb270a0:single at 20 GHz: +2.5 %
* dcpwg_s0.5_g0.5:even at 20 GHz: +3.5 %
* dcpwg_s1_g0.5:even at 20 GHz: +3.4 %
* board_ts_2d86371179:single at 20 GHz: +2.5 %
* board_vna_203c016d7c:single at 20 GHz: +2.4 %
* board_vna_abdc7a116a:single at 20 GHz: +2.4 %
* board_vna_d4a3c7d26e:single at 20 GHz: +2.4 %
* board_vna_a50c59cde9:single at 20 GHz: +2.2 %
* board_vna_d407391e05:single at 20 GHz: +2.3 %
* board_vna_a54b23cf0c:single at 20 GHz: +2.4 %
* board_vna_fe17140140:single at 20 GHz: +2.6 %
* board_vna_06c826e75e:single at 20 GHz: +2.5 %
* ms_u2_er10.2:single at 25.12 GHz: +2.1 %
* cpwg_u0.5_g0.25_mask:single at 25.12 GHz: -5.9 %
* cpwg_u0.5_g0.5_mask:single at 25.12 GHz: +5.7 %
* cpwg_u0.5_g1_mask:single at 25.12 GHz: +5.0 %
* cpwg_u0.5_g2:single at 25.12 GHz: +5.9 %
* cpwg_u0.5_g4:single at 25.12 GHz: +2.5 %
* cpwg_u1_g0.25_mask:single at 25.12 GHz: +5.5 %
* cpwg_u1_g0.5_mask:single at 25.12 GHz: +5.7 %
* cpwg_u1_g1_mask:single at 25.12 GHz: +4.5 %
* cpwg_u1_g2:single at 25.12 GHz: +4.2 %
* cpwg_u2_g0.25_mask:single at 25.12 GHz: +5.4 %
* cpwg_u2_g1_mask:single at 25.12 GHz: +3.7 %
* cpwg_u2_g2:single at 25.12 GHz: +2.8 %
* dcpwg_s0.5_g1:odd at 25.12 GHz: -3.6 %
* dcpwg_s1_g1:odd at 25.12 GHz: +3.1 %
* board_vna_941b481aea:single at 25.12 GHz: +2.9 %
* board_vna_8cfac7f6b8:single at 25.12 GHz: +2.9 %
* board_vna_1931634b2c:single at 25.12 GHz: +2.7 %
* board_vna_8eb88d4004:single at 25.12 GHz: +2.2 %
* board_vna_dea4295936:single at 25.12 GHz: +2.2 %
* board_vna_f010b60afd:single at 25.12 GHz: +2.3 %
* board_vna_d62f53732a:single at 25.12 GHz: +2.7 %
* board_vna_6e6ba2cdec:single at 25.12 GHz: +2.2 %
* board_tb_aisler4_assumed_a_38c85aad19:single at 25.12 GHz: +2.7 %
* board_tb_aisler4_assumed_a_46522088e3:single at 25.12 GHz: +2.5 %
* board_tb_aisler4_assumed_a_2746a11425:single at 25.12 GHz: +3.2 %
* board_tb_aisler4_assumed_a_2634154eb6:single at 25.12 GHz: +2.9 %
* board_tb_aisler4_assumed_a_125dd21c8e:single at 25.12 GHz: +2.7 %
* board_tb_jlc4_7628_a_f89c7c237c:single at 25.12 GHz: +3.8 %
* board_tb_jlc4_7628_a_b31b8f0d28:single at 25.12 GHz: +3.1 %
* board_tb_jlc4_7628_a_0a4cbab003:single at 25.12 GHz: +3.8 %
* board_tb_jlc4_7628_a_e84d0e9ef7:single at 25.12 GHz: +3.5 %
* board_tb_jlc4_7628_a_f819d49f18:single at 25.12 GHz: +3.3 %
* board_tb_jlc6_7628_a_881652e90e:single at 25.12 GHz: +3.1 %
* board_tb_jlc6_7628_a_4dcf76077e:single at 25.12 GHz: +3.8 %
* board_tb_jlc6_7628_a_abb5f8deeb:single at 25.12 GHz: +3.5 %
* board_tb_jlc6_7628_a_65eb75d980:single at 25.12 GHz: +3.3 %
* board_tb_jlc8_7628_a_1f46de1be4:single at 25.12 GHz: +3.1 %
* board_tb_jlc8_7628_a_3d0ed285bb:single at 25.12 GHz: +3.8 %
* board_tb_jlc8_7628_a_13f54cd4d6:single at 25.12 GHz: +3.5 %
* board_tb_jlc8_7628_a_02b4203d95:single at 25.12 GHz: +3.3 %
* ms_u1.5_er10.2:single at 25.12 GHz: +2.0 %
* ms_u3_er10.2:single at 25.12 GHz: +2.1 %
* cpwg_u0.5_g2_mask:single at 25.12 GHz: +3.0 %
* cpwg_u1_g2_mask:single at 25.12 GHz: +2.7 %
* cpwg_u2_g2_mask:single at 25.12 GHz: +2.2 %
* dcpwg_s0.5_g0.5:odd at 25.12 GHz: -4.2 %
* dcpwg_s0.5_g2:odd at 25.12 GHz: -2.2 %
* dcpwg_s0.5_g2:even at 25.12 GHz: +2.6 %
* dcpwg_s1_g0.5:odd at 25.12 GHz: +4.2 %
* dcpwg_s1_g2:odd at 25.12 GHz: +2.1 %
* dcpwg_s1_g2:even at 25.12 GHz: +2.5 %
* board_vna_2b3a791d31:single at 25.12 GHz: +2.3 %
* board_vna_e40479e733:single at 25.12 GHz: +3.1 %
* board_vna_516c2960ed:single at 25.12 GHz: +2.2 %
* board_vna_4cf672f3df:single at 25.12 GHz: +2.5 %
* board_vna_68adae1fee:single at 25.12 GHz: +2.5 %
* board_vna_c01f0b7600:single at 25.12 GHz: +2.8 %
* board_vna_77796753d7:single at 25.12 GHz: +2.8 %
* board_vna_c557dca26e:single at 25.12 GHz: +3.0 %
* board_vna_3c5033f3e7:single at 25.12 GHz: +2.3 %
* board_vna_fee7cc8cd1:single at 25.12 GHz: +3.0 %
* board_vna_d8139a1133:single at 25.12 GHz: +3.1 %
* board_vna_5264cf2ca3:single at 25.12 GHz: +2.9 %
* board_vna_b145ecee93:single at 25.12 GHz: +2.3 %
* ms_u0.5_er10.2:single at 31.62 GHz: +2.1 %
* ms_u1_er10.2:single at 31.62 GHz: +2.4 %
* ms_h0.8_u1.9:single at 31.62 GHz: +2.1 %
* sle_er3-4.4_r3:single at 31.62 GHz: -6.5 %
* cpwg_u1_g4:single at 31.62 GHz: -4.8 %
* cpwg_u2_g4:single at 31.62 GHz: +2.5 %
* board_ts_b2b33741d9:odd at 31.62 GHz: -2.6 %
* board_tb_jlc4_7628_c_4bc8befe37:single at 31.62 GHz: +2.5 %
* board_tb_jlc4_7628_c_54fbd10f14:single at 31.62 GHz: +2.5 %
* board_tb_jlc4_7628_d_68e35168e3:single at 31.62 GHz: +2.2 %
* board_tb_jlc4_7628_d_40651b7c02:single at 31.62 GHz: +2.2 %
* board_tb_jlc6_7628_c_59a3b02602:single at 31.62 GHz: +2.5 %
* board_tb_jlc6_7628_c_545613da4b:single at 31.62 GHz: +2.5 %
* board_tb_jlc6_7628_d_596bf3ef97:single at 31.62 GHz: +2.2 %
* board_tb_jlc6_7628_d_abae9f018e:single at 31.62 GHz: +2.2 %
* board_tb_jlc8_7628_c_d8bd7e500f:single at 31.62 GHz: +2.5 %
* board_tb_jlc8_7628_c_f050a179c6:single at 31.62 GHz: +2.5 %
* ms_u0.35_er10.2:single at 31.62 GHz: +2.0 %
* ms_u0.75_er10.2:single at 31.62 GHz: +2.3 %
* ms_h0.8_u0.5:single at 31.62 GHz: +2.2 %
* cpwg_u0.5_g4_mask:single at 31.62 GHz: -5.3 %
* cpwg_u1_g4_mask:single at 31.62 GHz: +2.7 %
* cpw_w0.6_g0.25:single at 31.62 GHz: +2.3 %
* cpw_w0.6_g0.4:single at 31.62 GHz: +2.6 %
* cut_w0.4_c2_r3:single at 31.62 GHz: +2.6 %
* board_ts_9c62699b7e:odd at 31.62 GHz: -2.7 %
* ms_u0.2_er10.2:single at 40 GHz: +2.5 %
* ms_u1_er6.15:single at 40 GHz: +2.1 %
* ms_u2_er6.15:single at 40 GHz: +2.2 %
* ms_u5_er10.2:single at 40 GHz: +2.1 %
* sle_er3.66-4.5_r3:single at 40 GHz: -3.2 %
* cpw_w0.3_g0.15:single at 40 GHz: +4.9 %
* cpw_w0.3_g0.4:single at 40 GHz: +2.6 %
* cut_w0.2_c4_r3:single at 40 GHz: +2.7 %
* board_ts_b06906e884:even at 40 GHz: -5.8 %
* board_ts_b2b33741d9:even at 40 GHz: +2.1 %
* board_tb_jlc6_3313_a_42b533a063:single at 40 GHz: +4.1 %
* ms_u0.1_er10.2:single at 40 GHz: +2.4 %
* ms_u0.75_er6.15:single at 40 GHz: +2.0 %
* ms_u1.5_er6.15:single at 40 GHz: +2.2 %
* ms_u3_er6.15:single at 40 GHz: +2.1 %
* cpwg_u2_g4_mask:single at 40 GHz: -5.0 %
* board_ts_9c62699b7e:even at 40 GHz: -5.9 %
* board_tb_jlc4_7628_f3_a4e1e30382:single at 40 GHz: +2.0 %
* board_tb_jlc4_7628_f3_7e689f5994:single at 40 GHz: +2.1 %

## lossy sweep vs CST lossy per case and frequency (error %)

| case:mode | f GHz | α | Re Zc | εeff |
|---|---:|---:|---:|---:|
| anchor_forsten_suite_10:even | 0.1 | +17.43 | +0.20 | -0.08 |
| anchor_forsten_suite_10:even | 1 | +11.27 | +0.03 | -0.17 |
| anchor_forsten_suite_10:even | 5 | +11.33 | -0.01 | -0.46 |
| anchor_forsten_suite_10:even | 10 | +11.11 | -0.02 | -0.48 |
| anchor_forsten_suite_10:even | 20 | +10.98 | -0.02 | -0.49 |
| anchor_forsten_suite_10:even | 40 | +10.98 | -0.04 | -0.50 |
| anchor_forsten_suite_10:odd | 0.1 | +25.29 | +0.40 | +0.23 |
| anchor_forsten_suite_10:odd | 1 | +16.56 | +0.07 | -0.14 |
| anchor_forsten_suite_10:odd | 5 | +17.21 | +0.01 | -0.59 |
| anchor_forsten_suite_10:odd | 10 | +17.35 | -0.01 | -0.62 |
| anchor_forsten_suite_10:odd | 20 | +17.71 | -0.02 | -0.65 |
| anchor_forsten_suite_10:odd | 40 | +18.04 | -0.02 | -0.68 |
| anchor_forsten_suite_1:single | 0.1 | +14.39 | +0.05 | -0.42 |
| anchor_forsten_suite_1:single | 1 | +12.40 | +0.07 | -0.48 |
| anchor_forsten_suite_1:single | 5 | +10.14 | -0.61 | -0.56 |
| anchor_forsten_suite_1:single | 10 | +8.14 | -2.39 | -0.55 |
| anchor_forsten_suite_1:single | 20 | +11.83 | -7.82 | -0.89 |
| anchor_forsten_suite_1:single | 40 | +16.32 | -14.86 | -0.68 |
| anchor_forsten_suite_2:single | 0.1 | +14.92 | -0.04 | -0.43 |
| anchor_forsten_suite_2:single | 1 | +12.50 | -0.06 | -0.35 |
| anchor_forsten_suite_2:single | 5 | +9.15 | -0.86 | -0.38 |
| anchor_forsten_suite_2:single | 10 | +7.83 | -2.42 | -0.39 |
| anchor_forsten_suite_2:single | 20 | +7.03 | -6.59 | -0.29 |
| anchor_forsten_suite_2:single | 40 | +8.69 | -11.79 | -1.09 |
| anchor_forsten_suite_3:single | 0.1 | +22.29 | +0.01 | -1.09 |
| anchor_forsten_suite_3:single | 1 | +19.30 | +0.05 | -1.08 |
| anchor_forsten_suite_4:single | 0.1 | +12.74 | +0.08 | -0.42 |
| anchor_forsten_suite_4:single | 1 | +10.33 | +0.01 | -0.34 |
| anchor_forsten_suite_4:single | 5 | +8.78 | -0.00 | -0.35 |
| anchor_forsten_suite_4:single | 10 | +8.19 | -0.34 | -0.63 |
| anchor_forsten_suite_4:single | 20 | +5.35 | -1.01 | -0.49 |
| anchor_forsten_suite_4:single | 40 | +2.98 | -3.72 | -0.40 |
| anchor_forsten_suite_5:single | 0.1 | +22.75 | +0.31 | -0.28 |
| anchor_forsten_suite_5:single | 1 | +16.12 | +0.11 | -0.69 |
| anchor_forsten_suite_5:single | 5 | +14.69 | -0.01 | -0.77 |
| anchor_forsten_suite_5:single | 10 | +15.65 | -2.14 | -0.86 |
| anchor_forsten_suite_6:even | 0.1 | +13.31 | +0.03 | -0.71 |
| anchor_forsten_suite_6:even | 1 | +12.31 | -0.11 | -1.06 |
| anchor_forsten_suite_6:even | 5 | +5.23 | -2.43 | -0.42 |
| anchor_forsten_suite_6:even | 10 | +4.50 | -6.46 | -0.54 |
| anchor_forsten_suite_6:even | 20 | +6.83 | -12.71 | -0.65 |
| anchor_forsten_suite_6:even | 40 | +10.42 | -19.36 | -0.72 |
| anchor_forsten_suite_6:odd | 0.1 | +23.01 | +0.07 | -1.14 |
| anchor_forsten_suite_6:odd | 1 | +24.99 | +0.02 | -1.97 |
| anchor_forsten_suite_6:odd | 5 | +16.05 | -0.75 | -0.82 |
| anchor_forsten_suite_6:odd | 10 | +19.24 | -5.07 | -1.10 |
| anchor_forsten_suite_6:odd | 20 | +33.04 | -22.97 | -1.44 |
| anchor_forsten_suite_6:odd | 40 | +52.19 | -45.61 | -0.95 |
| anchor_forsten_suite_7:even | 0.1 | +12.02 | +0.21 | -0.47 |
| anchor_forsten_suite_7:even | 1 | +9.09 | +0.10 | -0.55 |
| anchor_forsten_suite_7:even | 5 | +8.10 | +0.05 | -0.70 |
| anchor_forsten_suite_7:even | 10 | +7.16 | -0.20 | -0.69 |
| anchor_forsten_suite_7:even | 20 | +5.34 | -0.79 | -0.60 |
| anchor_forsten_suite_7:even | 40 | +3.32 | -2.42 | -0.67 |
| anchor_forsten_suite_7:odd | 0.1 | +15.30 | +0.22 | -0.56 |
| anchor_forsten_suite_7:odd | 1 | +11.70 | +0.08 | -0.69 |
| anchor_forsten_suite_7:odd | 5 | +11.14 | +0.04 | -0.83 |
| anchor_forsten_suite_7:odd | 10 | +10.63 | +0.00 | -0.91 |
| anchor_forsten_suite_7:odd | 20 | +8.98 | -0.24 | -0.86 |
| anchor_forsten_suite_7:odd | 40 | +5.90 | -1.80 | -0.96 |
| anchor_forsten_suite_8:even | 0.1 | +14.58 | +0.22 | -0.63 |
| anchor_forsten_suite_8:even | 1 | +11.62 | +0.10 | -0.92 |
| anchor_forsten_suite_8:even | 5 | +9.82 | -0.08 | -0.91 |
| anchor_forsten_suite_8:even | 10 | +7.34 | -2.28 | -0.87 |
| anchor_forsten_suite_8:even | 20 | +2.39 | -43.31 | -1.45 |
| anchor_forsten_suite_8:odd | 0.1 | +19.46 | +0.33 | -0.72 |
| anchor_forsten_suite_8:odd | 1 | +15.33 | +0.16 | -1.07 |
| anchor_forsten_suite_8:odd | 5 | +14.35 | +0.06 | -1.06 |
| anchor_forsten_suite_8:odd | 10 | +14.11 | -0.23 | -1.09 |
| anchor_forsten_suite_9:single | 0.1 | +20.11 | +0.15 | +0.10 |
| anchor_forsten_suite_9:single | 1 | +13.56 | -0.03 | -0.25 |
| anchor_forsten_suite_9:single | 5 | +12.08 | -0.07 | -0.34 |
| anchor_forsten_suite_9:single | 10 | +11.82 | -0.08 | -0.36 |
| anchor_forsten_suite_9:single | 20 | +11.86 | -0.10 | -0.40 |
| anchor_forsten_suite_9:single | 40 | +11.72 | -0.12 | -0.38 |
| board_ts_01a9401349:even | 0.1 | +13.16 | +0.29 | -0.20 |
| board_ts_01a9401349:even | 1 | +6.82 | +0.03 | -0.38 |
| board_ts_01a9401349:even | 5 | +5.75 | +0.04 | -0.59 |
| board_ts_01a9401349:even | 10 | +2.43 | -0.02 | -0.46 |
| board_ts_01a9401349:even | 20 | +3.77 | -0.11 | -0.62 |
| board_ts_01a9401349:even | 40 | -0.01 | -0.80 | -0.71 |
| board_ts_01a9401349:odd | 0.1 | +19.14 | +0.40 | -0.13 |
| board_ts_01a9401349:odd | 1 | +9.95 | +0.03 | -0.47 |
| board_ts_01a9401349:odd | 5 | +7.85 | +0.02 | -0.75 |
| board_ts_01a9401349:odd | 10 | +4.65 | -0.06 | -0.65 |
| board_ts_01a9401349:odd | 20 | +6.75 | -0.02 | -0.79 |
| board_ts_01a9401349:odd | 40 | +4.62 | -0.01 | -0.94 |
| board_ts_c472bed9a1:even | 0.1 | +18.61 | +0.18 | -0.61 |
| board_ts_c472bed9a1:even | 1 | +9.44 | +0.08 | -0.50 |
| board_ts_c472bed9a1:even | 5 | +7.21 | +0.03 | -0.67 |
| board_ts_c472bed9a1:even | 10 | +4.14 | -0.04 | -0.58 |
| board_ts_c472bed9a1:even | 20 | +6.33 | -0.01 | -0.69 |
| board_ts_c472bed9a1:even | 40 | +5.15 | -0.09 | -0.73 |
| board_ts_c472bed9a1:odd | 0.1 | +23.50 | +0.22 | -0.59 |
| board_ts_c472bed9a1:odd | 1 | +11.77 | +0.11 | -0.52 |
| board_ts_c472bed9a1:odd | 5 | +8.74 | +0.04 | -0.71 |
| board_ts_c472bed9a1:odd | 10 | +5.49 | -0.02 | -0.63 |
| board_ts_c472bed9a1:odd | 20 | +7.84 | +0.04 | -0.73 |
| board_ts_c472bed9a1:odd | 40 | +5.88 | +0.07 | -0.80 |
| board_vna_1d76ca6cc4:single | 0.1 | +20.62 | +0.12 | -1.05 |
| board_vna_1d76ca6cc4:single | 1 | +7.20 | -0.01 | -0.59 |
| board_vna_1d76ca6cc4:single | 5 | +5.58 | +0.01 | -0.69 |
| board_vna_1d76ca6cc4:single | 10 | +1.86 | -0.12 | -0.64 |
| board_vna_1d76ca6cc4:single | 20 | +5.38 | -0.09 | -0.88 |
| board_vna_1d76ca6cc4:single | 40 | +4.08 | -0.69 | -1.21 |
| board_vna_689da5c270:single | 0.1 | +12.06 | +0.37 | -0.52 |
| board_vna_689da5c270:single | 1 | +5.23 | +0.06 | -0.84 |
| board_vna_689da5c270:single | 5 | +4.13 | +0.05 | -1.02 |
| board_vna_689da5c270:single | 10 | -1.49 | -0.46 | -0.78 |
| board_vna_689da5c270:single | 20 | -11.13 | -12.75 | -0.89 |
| cpwg_u1_g0.5:single | 0.1 | +17.91 | +0.58 | -0.22 |
| cpwg_u1_g0.5:single | 1 | +8.43 | +0.15 | -0.82 |
| cpwg_u1_g0.5:single | 5 | +6.01 | +0.16 | -1.05 |
| cpwg_u1_g0.5:single | 10 | +0.84 | -0.19 | -0.63 |
| cpwg_u1_g0.5:single | 20 | -15.31 | -15.96 | -1.52 |
| dms_u1_s1:even | 0.1 | +10.03 | +0.38 | -0.55 |
| dms_u1_s1:even | 1 | +3.85 | +0.13 | -0.51 |
| dms_u1_s1:even | 5 | +3.38 | +0.19 | -0.69 |
| dms_u1_s1:even | 10 | -1.31 | +0.00 | -0.49 |
| dms_u1_s1:even | 20 | -0.62 | -0.49 | -0.72 |
| dms_u1_s1:even | 40 | -6.87 | -1.70 | -0.87 |
| dms_u1_s1:odd | 0.1 | +15.84 | +0.50 | -0.59 |
| dms_u1_s1:odd | 1 | +6.77 | +0.17 | -0.72 |
| dms_u1_s1:odd | 5 | +5.43 | +0.18 | -0.93 |
| dms_u1_s1:odd | 10 | +1.58 | +0.06 | -0.84 |
| dms_u1_s1:odd | 20 | +4.06 | +0.08 | -1.05 |
| dms_u1_s1:odd | 40 | -0.58 | -0.34 | -1.20 |
| dsl_u1_s1_r1:even | 0.1 | +16.58 | +0.19 | -0.66 |
| dsl_u1_s1_r1:even | 1 | +6.99 | +0.06 | -0.25 |
| dsl_u1_s1_r1:even | 5 | +5.55 | +0.07 | -0.48 |
| dsl_u1_s1_r1:even | 10 | +2.20 | -0.06 | -0.28 |
| dsl_u1_s1_r1:even | 20 | +5.42 | +0.04 | -0.53 |
| dsl_u1_s1_r1:even | 40 | +4.25 | +0.12 | -0.71 |
| dsl_u1_s1_r1:odd | 0.1 | +22.27 | +0.29 | -0.55 |
| dsl_u1_s1_r1:odd | 1 | +9.77 | +0.08 | -0.27 |
| dsl_u1_s1_r1:odd | 5 | +7.03 | +0.06 | -0.55 |
| dsl_u1_s1_r1:odd | 10 | +3.48 | -0.07 | -0.37 |
| dsl_u1_s1_r1:odd | 20 | +6.39 | +0.02 | -0.62 |
| dsl_u1_s1_r1:odd | 40 | +5.00 | +0.10 | -0.80 |
| ms_h0.2_u1.9:single | 0.1 | +9.27 | +0.48 | -1.40 |
| ms_h0.2_u1.9:single | 1 | +3.37 | +0.07 | -0.40 |
| ms_h0.2_u1.9:single | 5 | +3.32 | +0.13 | -0.56 |
| ms_h0.2_u1.9:single | 10 | -1.17 | +0.04 | -0.39 |
| ms_h0.2_u1.9:single | 20 | -0.46 | -0.05 | -1.53 |
| ms_h0.2_u1.9:single | 40 | -6.95 | -0.67 | -1.91 |
| ms_h1.6_u1.9:single | 0.1 | +3.76 | +0.26 | -2.63 |
| ms_h1.6_u1.9:single | 1 | -0.42 | +0.07 | -0.45 |
| ms_h1.6_u1.9:single | 5 | -6.87 | -0.71 | -2.62 |
| ms_h1.6_u1.9:single | 10 | -17.60 | -2.58 | -2.05 |
| ms_h1.6_u1.9:single | 20 | -21.88 | -7.41 | -1.00 |
| ms_h1.6_u1.9:single | 40 | -27.82 | -14.40 | -0.99 |
| ms_u0.5_er4.4:single | 0.1 | +13.79 | +0.37 | -0.46 |
| ms_u0.5_er4.4:single | 1 | +5.34 | +0.06 | -0.37 |
| ms_u0.5_er4.4:single | 5 | +4.23 | +0.21 | -0.54 |
| ms_u0.5_er4.4:single | 10 | -0.47 | +0.37 | -0.43 |
| ms_u0.5_er4.4:single | 20 | +0.36 | +1.28 | -0.63 |
| ms_u0.5_er4.4:single | 40 | -6.82 | +3.67 | -0.79 |
| ms_u2_er3.66:single | 0.1 | +12.17 | +0.22 | -0.76 |
| ms_u2_er3.66:single | 1 | +6.61 | +0.04 | -0.33 |
| ms_u2_er3.66:single | 5 | +5.81 | +0.13 | -0.88 |
| ms_u2_er3.66:single | 10 | +1.48 | +0.02 | -0.92 |
| ms_u2_er3.66:single | 20 | +0.36 | -0.38 | -0.93 |
| ms_u2_er3.66:single | 40 | -7.16 | -1.26 | -0.49 |
| ms_u2_er4.4:single | 0.1 | +7.10 | +0.27 | -0.53 |
| ms_u2_er4.4:single | 1 | +2.50 | +0.05 | -0.37 |
| ms_u2_er4.4:single | 5 | +2.48 | +0.12 | -0.50 |
| ms_u2_er4.4:single | 10 | -2.92 | -0.10 | -0.45 |
| ms_u2_er4.4:single | 20 | -2.88 | -0.54 | -0.52 |
| ms_u2_er4.4:single | 40 | -11.06 | -1.62 | -0.71 |
| sl_h0.1_u1_er4.2:single | 0.1 | +20.07 | +0.27 | -0.61 |
| sl_h0.1_u1_er4.2:single | 1 | +8.75 | +0.13 | -0.27 |
| sl_h0.1_u1_er4.2:single | 5 | +6.48 | +0.11 | -0.55 |
| sl_h0.1_u1_er4.2:single | 10 | +3.08 | -0.03 | -0.34 |
| sl_h0.1_u1_er4.2:single | 20 | +5.91 | +0.07 | -0.59 |
| sl_h0.1_u1_er4.2:single | 40 | +4.65 | +0.14 | -0.78 |

## FW vs CST outliers (> 1.0 %)

* board_ts_ea1df30155 E zdiff: +51.30 %
* board_ts_860e846530 E zdiff: +49.71 %
* board_ts_471a028ef5 E zdiff: +40.75 %
* board_ts_9c62699b7e E zdiff: +23.44 %
* board_ts_ea1df30155 E zeven: -14.53 %
* board_ts_b06906e884 E zdiff: +13.90 %
* board_ts_860e846530 E zeven: -13.80 %
* board_ts_6da2ee36b6 E zdiff: +12.68 %
* board_ts_b2b33741d9 E zdiff: +11.93 %
* board_ts_01906bbc96 E zdiff: +11.49 %
* board_ts_834d341d7a E zdiff: +10.87 %
* board_ts_471a028ef5 E zeven: -5.96 %
* board_ts_f6aa252eb9 E zdiff: +3.05 %
* board_ts_169daecc6a E zdiff: +2.14 %
* board_ts_54b3d93297 E zdiff: +2.13 %
* board_ts_9c62699b7e E zeven: -1.51 %
* edge_pair_r2 E zdiff: +1.21 %

