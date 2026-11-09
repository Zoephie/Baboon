float view_depth(float window) {
    float ndc = window * 2.0 - 1.0;
    return (ndc - u_depth.y) / (u_depth.x - ndc * u_depth.z);
}
