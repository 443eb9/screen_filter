
Texture2D screenTexture : register(t0);
SamplerState samplerState : register(s0);

float gamma_to_linear(float x) {
    if (x <= 0.0) return x;
    if (x <= 0.04045) return x / 12.92;
    return pow((x + 0.055) / 1.055, 2.4);
}

float4 main(float4 pos : SV_Position, float2 tex : TEXCOORD) : SV_Target {
    float4 color = screenTexture.Sample(samplerState, tex);
    float red = gamma_to_linear(color.r);
    float green = gamma_to_linear(color.g);
    float blue = gamma_to_linear(color.b);

    float l = 0.41222146 * red + 0.53633255 * green + 0.051445995 * blue;
    float m = 0.2119035 * red + 0.6806995 * green + 0.10739696 * blue;
    float s = 0.08830246 * red + 0.28171885 * green + 0.6299787 * blue;
    float l_ = pow(l, 1.0 / 3.0);
    float m_ = pow(m, 1.0 / 3.0);
    float s_ = pow(s, 1.0 / 3.0);
    float okl = 0.21045426 * l_ + 0.7936178 * m_ - 0.004072047 * s_;

    return float4(okl, okl, okl, 1.0f);
}
