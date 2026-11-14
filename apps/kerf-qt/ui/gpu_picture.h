#pragma once

#include <QtGlobal>

// Drawing the viewer's picture on the GPU needs QRhiWidget, from Qt 6.7. With
// an older Qt the viewer draws it on the CPU, as it always has.
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
#define KERF_GPU_PICTURE 1

#include <QImage>
#include <QRectF>
#include <QRhiWidget>

#include <memory>

class QRhi;
class QRhiBuffer;
class QRhiGraphicsPipeline;
class QRhiSampler;
class QRhiShaderResourceBindings;
class QRhiTexture;

namespace kerf {

// A frame drawn on the GPU: the part of it that falls in this widget, sampled
// between its pixels. The viewer lays it over its picture where nothing rounds
// it or is drawn over it, and draws the rest itself. A new frame is uploaded
// once; changing the widget's size or the part shown draws it again from that.
class GpuPicture final : public QRhiWidget {
public:
    explicit GpuPicture(QWidget *parent = nullptr);
    ~GpuPicture() override;

    // Whether the window system puts windows together on the GPU, and the device
    // QRhiWidget would draw with starts here: Metal on macOS, Direct3D 11 on
    // Windows, OpenGL elsewhere. Asked once. Where not, no GpuPicture is made: one
    // in a window turns all of it over to the GPU.
    static bool available();
    // For tests: what available() answers, whatever the machine; -1 asks it.
    static inline int s_availableForTests = -1;

    void setFrame(const QImage &frame);
    // What of the frame this widget shows, as fractions of it: (0, 0, 1, 1) is
    // all of it.
    void setSource(const QRectF &source);
    QRectF source() const { return m_source; }
    // Frames uploaded so far, for tests.
    int uploads() const { return m_uploads; }

protected:
    void initialize(QRhiCommandBuffer *cb) override;
    void render(QRhiCommandBuffer *cb) override;
    void releaseResources() override;

private:
    QImage m_frame;
    bool m_newFrame = false;
    QRectF m_source{0.0, 0.0, 1.0, 1.0};
    int m_uploads = 0;
    QRhi *m_rhi = nullptr;
    std::unique_ptr<QRhiBuffer> m_vertices;
    std::unique_ptr<QRhiBuffer> m_uniforms;
    std::unique_ptr<QRhiTexture> m_texture;
    std::unique_ptr<QRhiSampler> m_sampler;
    std::unique_ptr<QRhiShaderResourceBindings> m_bindings;
    std::unique_ptr<QRhiGraphicsPipeline> m_pipeline;
    bool m_verticesUploaded = false;
};

} // namespace kerf

#endif
