
Texture2D screenTexture : register(t0);
SamplerState samplerState : register(s0);

// Color model converting code is translated from `bevy` project.
// Here's the original license:
// 
// MIT License

// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:

// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

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
