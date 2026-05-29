#!/usr/bin/env python3
"""
GPU intensive computation using OpenGL
Install: pip install PyOpenGL PyOpenGL_accelerate glfw
Usage:
    joule-profiler --gpu profile -- python3 gpu.py {n}
    with n = 10 < 1s
         n = 100 < 5s
         n = 1000 < 1m
"""


import sys
import numpy as np
from OpenGL.GL import *
from OpenGL.GL import shaders
import glfw

# Fragment shader avec calculs intensifs
FRAGMENT_SHADER = """
#version 330 core
out vec4 FragColor;
uniform float time;
uniform int iterations;

void main() {
    vec2 uv = gl_FragCoord.xy / 1024.0;
    vec3 color = vec3(0.0);
    
    // Calculs GPU intensifs - Mandelbrot-like
    for (int i = 0; i < iterations; i++) {
        float angle = float(i) * 0.1 + time;
        vec2 c = vec2(cos(angle), sin(angle)) * 0.5;
        vec2 z = uv - 0.5;
        
        for (int j = 0; j < 100; j++) {
            z = vec2(z.x*z.x - z.y*z.y, 2.0*z.x*z.y) + c;
            if (length(z) > 2.0) break;
            color += vec3(0.01);
        }
    }
    
    FragColor = vec4(color, 1.0);
}
"""

VERTEX_SHADER = """
#version 330 core
layout (location = 0) in vec2 aPos;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
}
"""

def main():
    if len(sys.argv) < 2:
        print("Usage: python3 gpu_compute.py <iterations>")
        sys.exit(1)
    
    iterations = int(sys.argv[1])
    
    print("__GPU_START__", flush=True)
    
    if not glfw.init():
        print("Failed to initialize GLFW")
        sys.exit(1)
    
    glfw.window_hint(glfw.CONTEXT_VERSION_MAJOR, 3)
    glfw.window_hint(glfw.CONTEXT_VERSION_MINOR, 3)
    glfw.window_hint(glfw.OPENGL_PROFILE, glfw.OPENGL_CORE_PROFILE)
    glfw.window_hint(glfw.VISIBLE, glfw.FALSE)  # Invisible
    
    window = glfw.create_window(1024, 768, "GPU Compute", None, None)
    if not window:
        glfw.terminate()
        sys.exit(1)
    
    glfw.make_context_current(window)
    
    vertex = shaders.compileShader(VERTEX_SHADER, GL_VERTEX_SHADER)
    fragment = shaders.compileShader(FRAGMENT_SHADER, GL_FRAGMENT_SHADER)
    program = shaders.compileProgram(vertex, fragment)
    
    vertices = np.array([
        -1.0, -1.0,
         1.0, -1.0,
        -1.0,  1.0,
         1.0,  1.0,
    ], dtype=np.float32)
    
    vao = glGenVertexArrays(1)
    vbo = glGenBuffers(1)
    glBindVertexArray(vao)
    glBindBuffer(GL_ARRAY_BUFFER, vbo)
    glBufferData(GL_ARRAY_BUFFER, vertices.nbytes, vertices, GL_STATIC_DRAW)
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, None)
    glEnableVertexAttribArray(0)
    
    glUseProgram(program)
    time_loc = glGetUniformLocation(program, "time")
    iter_loc = glGetUniformLocation(program, "iterations")
    glUniform1i(iter_loc, iterations)
    
    for frame in range(100):
        glClear(GL_COLOR_BUFFER_BIT)
        glUniform1f(time_loc, frame * 0.1)
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4)
        glfw.swap_buffers(window)
        glFinish()
    
    print("__GPU_END__", flush=True)
    
    glfw.terminate()

if __name__ == "__main__":
    main()