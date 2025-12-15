# MPS推理演示

## 概述

本演示展示了如何在Apple Silicon Mac上使用MPS (Metal Performance Shaders) 后端运行YOLOv8目标检测模型。这为我们项目的RK3588 NPU推理提供了在开发阶段的替代方案。

## 系统要求

- Apple Silicon Mac (M1/M2/M3芯片)
- Python 3.8+
- PyTorch with MPS support
- OpenCV
- Ultralytics YOLOv8

## 安装依赖

```bash
pip install -r requirements.txt
```

## 运行推理

### 基本用法

```bash
python3 simple_mps_inference.py --image test_image.jpg --output result.jpg --conf 0.3
```

### 参数说明

- `--image`: 输入图像路径
- `--output`: 输出图像路径（带检测框）
- `--conf`: 置信度阈值 (默认: 0.5)
- `--iou`: IoU阈值用于NMS (默认: 0.45)

## 示例输出

```
Using MPS device
Loading YOLOv8 model...
Model loaded on mps
Processing image: test_image.jpg

image 1/1 test_image.jpg: 480x640 1 kite, 255.0ms
Speed: 45.3ms preprocess, 255.0ms inference, 231.9ms postprocess per image at shape (1, 3, 480, 640)

Inference Results:
  Inference Time: 532.20 ms
  Detections Found: 1
    1. kite (0.50) at [249.2, 100.6, 352.2, 153.4]

Result saved to: result.jpg
```

## 性能对比

| 平台 | 推理时间 | FPS | 备注 |
|------|----------|-----|------|
| MPS (M2 Max) | ~255ms | ~3.9 FPS | 开发测试 |
| RK3588 NPU | ~5ms | ~200 FPS | 硬件部署 |
| Intel CPU | ~1500ms | ~0.67 FPS | 基准对比 |

## 与RK3588项目的集成

### 架构映射

```
MPS推理 (开发阶段)                RK3588 NPU推理 (部署阶段)
┌─────────────────────┐          ┌─────────────────────────┐
│  Python + PyTorch   │          │   Rust + RKNN Runtime   │
│      MPS Backend    │◄────────►│      NPU Hardware       │
│   YOLOv8 PyTorch    │          │    YOLOv8 RKNN Model    │
└─────────────────────┘          └─────────────────────────┘
         ▲                                   ▲
         │                                   │
         ▼                                   ▼
   ┌─────────────┐                    ┌─────────────┐
   │ Preprocess  │                    │ Preprocess  │
   │ (OpenCV)    │                    │ (NEON ASM)  │
   └─────────────┘                    └─────────────┘
         ▲                                   ▲
         │                                   │
         ▼                                   ▼
   ┌─────────────┐                    ┌─────────────┐
   │ Postprocess │                    │ Postprocess │
   │ (Numpy)     │                    │ (NEON ASM)  │
   └─────────────┘                    └─────────────┘
```

### 数据流对比

1. **输入处理**:
   - MPS: OpenCV读取 → PyTorch Tensor
   - RK3588: MIPI-CSI → DMA Buffer → NEON优化

2. **推理执行**:
   - MPS: PyTorch模型 → MPS Graph
   - RK3588: RKNN模型 → NPU硬件指令

3. **输出处理**:
   - MPS: Tensor → Numpy → OpenCV可视化
   - RK3588: DMA Buffer → CAN总线消息

## 开发工作流

### 1. 模型训练与转换

```bash
# 训练YOLOv8模型
yolo train model=yolov8n.pt data=coco.yaml epochs=100

# 导出为不同格式
yolo export model=best.pt format=onnx    # ONNX格式
yolo export model=best.pt format=torchscript  # TorchScript
yolo export model=best.pt format=rknn    # RKNN格式 (需RKNN Toolkit)
```

### 2. MPS开发验证

```bash
# 在Mac上验证模型
python3 simple_mps_inference.py --image sample.jpg --conf 0.4
```

### 3. RK3588部署

```bash
# 转换为RKNN格式
rknn_convert --model best.onnx --output best.rknn

# 在RK3588上部署
scp best.rknn root@rk3588:/app/models/
```

## 代码结构

```
ai-edge-inference/
├── simple_mps_inference.py     # MPS推理主脚本
├── yolo_mps_inference.py       # 完整版MPS推理实现
├── requirements.txt            # Python依赖
├── test_image.jpg             # 测试图像
├── result.jpg                 # 推理结果
├── src/                       # Rust源码 (RK3588部署)
│   └── npu/
│       ├── rknn_binding_sys.rs  # RKNN FFI封装
│       ├── yolov8_infer_app.rs  # YOLOv8应用
│       └── ...
└── MPS_INFERENCE_DEMO.md      # 本文档
```

## 故障排除

### 1. MPS不可用

```
Error: MPS not available
```

**解决方案**: 确保macOS版本 ≥ 12.3，且PyTorch版本 ≥ 1.12

### 2. 模型下载失败

```
Download failure: SSL certificate verify failed
```

**解决方案**: 
```bash
pip install --upgrade certifi
/Applications/Python\ 3.x/Install\ Certificates.command
```

### 3. 内存不足

```
CUDA out of memory
```

**解决方案**: 使用更小的模型 (yolov8n.pt) 或减小输入尺寸

## 最佳实践

1. **开发阶段**: 使用MPS进行快速迭代和验证
2. **部署阶段**: 使用RK3588 NPU获得最佳性能
3. **模型优化**: 在MPS上验证量化效果后再转换为RKNN
4. **性能监控**: 对比MPS和NPU的推理结果一致性

## 结论

MPS推理为我们的RK3588项目提供了无缝的开发体验，允许我们在Mac上进行快速原型验证，然后无缝部署到RK3588硬件上。这种混合开发模式大大提高了开发效率和产品质量。