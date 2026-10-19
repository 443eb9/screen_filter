cbuffer Parameters : register(b0) {
    uint rotation;
    uint3 padding;
};

void main(uint id : SV_VertexID, out float4 position : SV_Position, out float2 tex : TEXCOORD) {
    float2 uv = float2((id << 1) & 2, id & 2);
    position = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    if (rotation == 1) tex = float2(uv.y, 1.0 - uv.x);
    else if (rotation == 2) tex = 1.0 - uv;
    else if (rotation == 3) tex = float2(1.0 - uv.y, uv.x);
    else tex = uv;
}
