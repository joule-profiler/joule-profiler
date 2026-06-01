/*
  Source: https://github.com/canonizer/mandelbrot-dyn

  Compile: nvcc -O3 -Xcompiler -fopenmp mandelbrot.cu -o mandelbrot
  Usage: joule-profiler --gpu profile -- ./mandelbrot {n}
*/

/*
  The MIT License (MIT)

  Copyright (c) 2014 Andrew V. Adinetz

  Permission is hereby granted, free of charge, to any person obtaining a copy
  of this software and associated documentation files (the "Software"), to deal
  in the Software without restriction, including without limitation the rights
  to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  copies of the Software, and to permit persons to whom the Software is
  furnished to do so, subject to the following conditions:

  The above copyright notice and this permission notice shall be included in all
  copies or substantial portions of the Software.

  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
  IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
  AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
  SOFTWARE.
*/

/** @file histo-global.cu histogram with global memory atomics */

#include <omp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/** CUDA check macro */
#define cucheck(call)                                                          \
  {                                                                            \
    cudaError_t res = (call);                                                  \
    if (res != cudaSuccess) {                                                  \
      const char *err_str = cudaGetErrorString(res);                           \
      fprintf(stderr, "%s (%d): %s in %s", __FILE__, __LINE__, err_str,        \
              #call);                                                          \
      exit(-1);                                                                \
    }                                                                          \
  }

/** time spent in device */
double gpu_time = 0;

/** a useful function to compute the number of threads */
int divup(int x, int y) { return x / y + (x % y ? 1 : 0); }

/** a simple complex type */
struct complex {
  __host__ __device__ complex(float re, float im = 0) {
    this->re = re;
    this->im = im;
  }
  /** real and imaginary part */
  float re, im;
}; // struct complex

// operator overloads for complex numbers
inline __host__ __device__ complex operator+(const complex &a,
                                             const complex &b) {
  return complex(a.re + b.re, a.im + b.im);
}
inline __host__ __device__ complex operator-(const complex &a) {
  return complex(-a.re, -a.im);
}
inline __host__ __device__ complex operator-(const complex &a,
                                             const complex &b) {
  return complex(a.re - b.re, a.im - b.im);
}
inline __host__ __device__ complex operator*(const complex &a,
                                             const complex &b) {
  return complex(a.re * b.re - a.im * b.im, a.im * b.re + a.re * b.im);
}
inline __host__ __device__ float abs2(const complex &a) {
  return a.re * a.re + a.im * a.im;
}
inline __host__ __device__ complex operator/(const complex &a,
                                             const complex &b) {
  float invabs2 = 1 / abs2(b);
  return complex((a.re * b.re + a.im * b.im) * invabs2,
                 (a.im * b.re - b.im * a.re) * invabs2);
} // operator/

#define MAX_DWELL 256
#define BS 256
/** computes the dwell for a single pixel */
__device__ int pixel_dwell(int w, int h, complex cmin, complex cmax, int x,
                           int y) {
  complex dc = cmax - cmin;
  float fx = (float)x / w, fy = (float)y / h;
  complex c = cmin + complex(fx * dc.re, fy * dc.im);
  int dwell = 0;
  complex z = c;
  while (dwell < MAX_DWELL && abs2(z) < 2 * 2) {
    z = z * z + c;
    dwell++;
  }
  return dwell;
} // pixel_dwell

/** computes the dwells for Mandelbrot image
                @param dwells the output array
                @param w the width of the output image
                @param h the height of the output image
                @param cmin the complex value associated with the left-bottom
   corner of the image
                @param cmax the complex value associated with the right-top
   corner of the image
 */
__global__ void mandelbrot_k(int *dwells, int w, int h, complex cmin,
                             complex cmax) {
  // complex value to start iteration (c)
  int x = threadIdx.x + blockIdx.x * blockDim.x;
  int y = threadIdx.y + blockIdx.y * blockDim.y;
  int dwell = pixel_dwell(w, h, cmin, cmax, x, y);
  dwells[y * w + x] = dwell;
} // mandelbrot_k

/** data size */
#define H (16 * 1024)
#define W (16 * 1024)
#define IMAGE_PATH "./mandelbrot.png"

int main(int argc, char **argv) {
  // n = number of times to recompute the image inside the timed region.
  // Defaults to 1; pass on the command line, e.g.  ./mandelbrot 10
  int n = 1;
  if (argc > 1) {
    n = atoi(argv[1]);
    if (n < 1)
      n = 1;
  }

  // allocate memory
  int w = W, h = H;
  size_t dwell_sz = w * h * sizeof(int);
  int *h_dwells, *d_dwells;
  cucheck(cudaMalloc((void **)&d_dwells, dwell_sz));
  h_dwells = (int *)malloc(dwell_sz);

  // compute the dwells, copy them back
  printf("__GPU_START__\n");
  fflush(stdout);
  double t1 = omp_get_wtime();
  dim3 bs(64, 4), grid(divup(w, bs.x), divup(h, bs.y));
  for (int i = 0; i < n; i++) {
    mandelbrot_k<<<grid, bs>>>(d_dwells, w, h, complex(-1.5, -1),
                               complex(0.5, 1));
  }
  cucheck(cudaDeviceSynchronize());
  double t2 = omp_get_wtime();
  printf("__GPU_END__\n");
  fflush(stdout);
  cucheck(cudaMemcpy(h_dwells, d_dwells, dwell_sz, cudaMemcpyDeviceToHost));
  gpu_time = t2 - t1;

  // print performance (Mpix/s accounts for the n repeats)
  printf("Mandelbrot set computed %d time(s) in %.3lf s, at %.3lf Mpix/s\n", n,
         gpu_time, (double)n * h * w * 1e-6 / gpu_time);

  // free data
  cudaFree(d_dwells);
  free(h_dwells);
  return 0;
} // main
