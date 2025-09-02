
Texture2D screenTexture : register(t0);
SamplerState samplerState : register(s0);

float gamma_to_linear(float x) {
    if (x <= 0.0) return x;
    if (x <= 0.04045) return x / 12.92;
    return pow((x + 0.055) / 1.055, 2.4);
}

float4 main(float4 pos : SV_Position, float2 tex : TEXCOORD) : SV_Target {
    float4 color = screenTexture.Sample(samplerState, tex);
    float r = gamma_to_linear(color.r);
    float g = gamma_to_linear(color.g);
    float b = gamma_to_linear(color.b);

    float y = r * 0.2126729 + g * 0.7151522 + b * 0.072175;
    const float CIE_EPSILON = 216.0 / 24389.0;
    const float CIE_KAPPA = 24389.0 / 27.0;
    float fy = y > CIE_EPSILON ? pow(y, 1.0 / 3.0) : (CIE_KAPPA * y + 16.0) / 116.0;
    float l = 1.16 * fy - 0.16;
    return float4(l, l, l, 1.0f);
}
