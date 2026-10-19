// Oklab matrices from Bjorn Ottosson's reference implementation.
// https://bottosson.github.io/posts/oklab/
// Color model conversion derived from Bevy, under the MIT License.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

Texture2D<float4> screenTexture : register(t0);
SamplerState samplerState : register(s0);

float signed_cbrt(float value) {
    return sign(value) * pow(abs(value), 1.0f / 3.0f);
}

float4 main(float4 position : SV_Position, float2 tex : TEXCOORD) : SV_Target {
    float3 rgb = screenTexture.SampleLevel(samplerState, tex, 0).rgb;
    float l = dot(rgb, float3(0.4122214708f, 0.5363325363f, 0.0514459929f));
    float m = dot(rgb, float3(0.2119034982f, 0.6806995451f, 0.1073969566f));
    float s = dot(rgb, float3(0.0883024619f, 0.2817188376f, 0.6299787005f));
    float lightness = 0.2104542553f * signed_cbrt(l)
                    + 0.7936177850f * signed_cbrt(m)
                    - 0.0040720468f * signed_cbrt(s);

    // Oklab (L, 0, 0) maps to the linear RGB neutral (L^3, L^3, L^3).
    float gray = lightness * lightness * lightness;
    return float4(gray, gray, gray, 1.0f);
}
