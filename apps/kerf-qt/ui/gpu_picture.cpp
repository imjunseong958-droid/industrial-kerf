#include "gpu_picture.h"

#ifdef KERF_GPU_PICTURE

#include <QFile>
#include <QMatrix4x4>

#include <QOffscreenSurface>

#include <rhi/qrhi.h>
#include <rhi/qrhi_platform.h>

#include <cstring>

namespace kerf {

namespace {

QShader loadShader(const QString &name) {
    QFile file(name);
    return file.open(QIODevice::ReadOnly) ? QShader::fromSerialized(file.readAll()) : QShader();
}

// The widget as two triangles, x and y from 0 to 1, from its top left.
constexpr float kQuad[] = {0.0f, 0.0f, 0.0f, 1.0f, 1.0f, 0.0f, 1.0f, 1.0f};
// A matrix, and the part of the frame shown.
constexpr quint32 kUniformBytes = 64 + 16;

} // namespace

bool GpuPicture::available() {
    if (s_availableForTests >= 0) return s_availableForTests == 1;
    static const bool starts = [] {
#if defined(Q_OS_MACOS)
        QRhiMetalInitParams params;
        return QRhi::probe(QRhi::Metal, &params);
#elif defined(Q_OS_WIN)
        QRhiD3D11InitParams params;
        return QRhi::probe(QRhi::D3D11, &params);
#else
        QRhiGles2InitParams params;
        std::unique_ptr<QOffscreenSurface> surface(QRhiGles2InitParams::newFallbackSurface());
        params.fallbackSurface = surface.get();
        return surface && surface->isValid() && QRhi::probe(QRhi::OpenGLES2, &params);
#endif
    }();
    return starts;
}

GpuPicture::GpuPicture(QWidget *parent) : QRhiWidget(parent) {
    setObjectName(QStringLiteral("gpuPicture"));
    setAttribute(Qt::WA_TransparentForMouseEvents);
}

GpuPicture::~GpuPicture() = default;

void GpuPicture::setFrame(const QImage &frame) {
    m_frame = frame;
    m_newFrame = true;
    update();
}

void GpuPicture::setSource(const QRectF &source) {
    if (source == m_source) return;
    m_source = source;
    update();
}

void GpuPicture::initialize(QRhiCommandBuffer *) {
    // Built once for a device; a new size keeps all of it.
    if (m_rhi != rhi()) {
        m_pipeline.reset();
        m_rhi = rhi();
    }
    if (m_pipeline) return;
    QRhi *r = m_rhi;
    m_vertices.reset(r->newBuffer(QRhiBuffer::Immutable, QRhiBuffer::VertexBuffer, sizeof(kQuad)));
    m_vertices->create();
    m_verticesUploaded = false;
    m_uniforms.reset(r->newBuffer(QRhiBuffer::Dynamic, QRhiBuffer::UniformBuffer, kUniformBytes));
    m_uniforms->create();
    m_sampler.reset(r->newSampler(QRhiSampler::Linear, QRhiSampler::Linear, QRhiSampler::None, QRhiSampler::ClampToEdge,
                                  QRhiSampler::ClampToEdge));
    m_sampler->create();
    // A pixel until the first frame; a frame of another size makes it again.
    m_texture.reset(r->newTexture(QRhiTexture::RGBA8, QSize(1, 1)));
    m_texture->create();
    m_newFrame = !m_frame.isNull();
    m_bindings.reset(r->newShaderResourceBindings());
    m_bindings->setBindings({QRhiShaderResourceBinding::uniformBuffer(0, QRhiShaderResourceBinding::VertexStage, m_uniforms.get()),
                             QRhiShaderResourceBinding::sampledTexture(1, QRhiShaderResourceBinding::FragmentStage, m_texture.get(),
                                                                       m_sampler.get())});
    m_bindings->create();
    m_pipeline.reset(r->newGraphicsPipeline());
    m_pipeline->setTopology(QRhiGraphicsPipeline::TriangleStrip);
    m_pipeline->setShaderStages({{QRhiShaderStage::Vertex, loadShader(QStringLiteral(":/shaders/picture.vert.qsb"))},
                                 {QRhiShaderStage::Fragment, loadShader(QStringLiteral(":/shaders/picture.frag.qsb"))}});
    QRhiVertexInputLayout layout;
    layout.setBindings({{2 * sizeof(float)}});
    layout.setAttributes({{0, 0, QRhiVertexInputAttribute::Float2, 0}});
    m_pipeline->setVertexInputLayout(layout);
    m_pipeline->setShaderResourceBindings(m_bindings.get());
    m_pipeline->setRenderPassDescriptor(renderTarget()->renderPassDescriptor());
    m_pipeline->create();
}

void GpuPicture::render(QRhiCommandBuffer *cb) {
    QRhiResourceUpdateBatch *updates = m_rhi->nextResourceUpdateBatch();
    if (!m_verticesUploaded) {
        updates->uploadStaticBuffer(m_vertices.get(), kQuad);
        m_verticesUploaded = true;
    }
    if (m_newFrame && !m_frame.isNull()) {
        const QImage image =
            m_frame.format() == QImage::Format_RGBA8888 ? m_frame : m_frame.convertToFormat(QImage::Format_RGBA8888);
        if (m_texture->pixelSize() != image.size()) {
            m_texture->setPixelSize(image.size());
            m_texture->create();
            m_bindings->create();
        }
        updates->uploadTexture(m_texture.get(), image);
        m_newFrame = false;
        ++m_uploads;
    }
    // The quad's 0..1, top down, to the device's clip space.
    QMatrix4x4 mvp = m_rhi->clipSpaceCorrMatrix();
    mvp.translate(-1.0f, 1.0f);
    mvp.scale(2.0f, -2.0f);
    float data[20];
    std::memcpy(data, mvp.constData(), 64);
    data[16] = float(m_source.left());
    data[17] = float(m_source.top());
    data[18] = float(m_source.right());
    data[19] = float(m_source.bottom());
    updates->updateDynamicBuffer(m_uniforms.get(), 0, kUniformBytes, data);

    const QSize size = renderTarget()->pixelSize();
    cb->beginPass(renderTarget(), Qt::black, {1.0f, 0}, updates);
    cb->setGraphicsPipeline(m_pipeline.get());
    cb->setViewport({0.0f, 0.0f, float(size.width()), float(size.height())});
    cb->setShaderResources();
    const QRhiCommandBuffer::VertexInput input(m_vertices.get(), 0);
    cb->setVertexInput(0, 1, &input);
    cb->draw(4);
    cb->endPass();
}

void GpuPicture::releaseResources() {
    m_pipeline.reset();
    m_bindings.reset();
    m_sampler.reset();
    m_texture.reset();
    m_uniforms.reset();
    m_vertices.reset();
    m_rhi = nullptr;
}

} // namespace kerf

#endif
