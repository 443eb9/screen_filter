
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

float toe(float x) {
    const float K1 = 0.206;
    const float K2 = 0.03;
    const float K3 = (1.0 + K1) / (1.0 + K2);
    return 0.5 * (K3 * x - K1 + sqrt((K3 * x - K1) * (K3 * x - K1) + 4.0 * K2 * K3 * x));
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
    okl = toe(okl);

    return float4(okl, okl, okl, 1.0f);
}
